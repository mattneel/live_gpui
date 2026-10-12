defmodule LiveGPUI.Template do
  @moduledoc "Compile-time compiler for the explicit native ~G template language."
  alias LiveGPUI.Node

  @kinds ~w(column row list text button input)

  def compile(source, caller) when is_binary(source) do
    state = %{rest: source, line: caller.line, file: caller.file}
    {root, state} = state |> whitespace() |> element()
    state = whitespace(state)
    unless state.rest == "", do: error!(state, "expected exactly one native root node")
    attrs = root.attrs

    if Map.has_key?(attrs, ":for") or Map.has_key?(attrs, ":if"),
      do: error!(state, "the native root must be unconditional; place directives on children")

    root = %{root | attrs: Map.put_new(attrs, "id", {:literal, "root"})}
    emit(root, caller)
  end

  def text!(value) when is_binary(value), do: value
  def text!(value) when is_number(value) or is_boolean(value), do: to_string(value)
  def text!(nil), do: ""
  def text!(_), do: raise(ArgumentError, "native text interpolation must be a scalar")

  def repeated!(%Node{id: id} = node) when is_binary(id) and byte_size(id) > 0, do: node
  def repeated!(_), do: raise(ArgumentError, "repeated native nodes require a nonempty stable id")

  defp element(%{rest: "<" <> _} = state) do
    line = state.line
    state = consume(state, "<")
    {kind, state} = name(state)
    unless kind in @kinds, do: error!(state, "unsupported native tag <#{kind}>")
    {attrs, state, self_closing?} = attributes(state, kind, %{})

    {children, state} =
      if self_closing? do
        {[], state}
      else
        if kind == "input", do: error!(state, "input must be self-closing")
        content(state, kind, [])
      end

    {%{kind: kind, attrs: attrs, children: children, line: line}, state}
  end

  defp element(state), do: error!(state, "expected a native element")

  defp attributes(state, kind, attrs) do
    before_space = state.rest
    state = whitespace(state)

    case state.rest do
      "/>" <> _ ->
        {attrs, consume(state, "/>"), true}

      ">" <> _ ->
        {attrs, consume(state, ">"), false}

      "" ->
        error!(state, "unterminated <#{kind}> tag")

      _ ->
        if before_space == state.rest,
          do: error!(state, "attributes must be separated by whitespace")

        attribute(state, kind, attrs)
    end
  end

  defp attribute(state, kind, attrs) do
    {key, state} = name(state)

    unless key in ["id", ":for", ":if"] or Node.allowed_property?(kind, key),
      do: error!(state, "unsupported #{kind} attribute #{inspect(key)}")

    if Map.has_key?(attrs, key), do: error!(state, "duplicate attribute #{inspect(key)}")
    state = whitespace(state)
    unless String.starts_with?(state.rest, "="), do: error!(state, "expected = after #{key}")
    state = state |> consume("=") |> whitespace()
    {value, state} = attribute_value(state)

    if key in [":for", ":if"] and elem(value, 0) != :expression,
      do: error!(state, "#{key} requires a brace expression")

    attributes(state, kind, Map.put(attrs, key, value))
  end

  defp attribute_value(%{rest: "{" <> _} = state), do: expression(state)

  defp attribute_value(%{rest: <<quote, _::binary>>} = state) when quote in [34, 39] do
    state = consume(state, <<quote>>)

    case :binary.match(state.rest, <<quote>>) do
      {length, 1} ->
        value = binary_part(state.rest, 0, length)
        {{:literal, value}, consume(state, value <> <<quote>>)}

      :nomatch ->
        error!(state, "unterminated quoted attribute")
    end
  end

  defp attribute_value(state),
    do: error!(state, "attribute values must be quoted strings or brace expressions")

  defp content(%{rest: "</" <> _} = state, kind, children) do
    state = consume(state, "</")
    {closing, state} = name(state)
    unless closing == kind, do: error!(state, "expected </#{kind}>, got </#{closing}>")
    state = whitespace(state)
    unless String.starts_with?(state.rest, ">"), do: error!(state, "expected > after closing tag")
    {Enum.reverse(children), consume(state, ">")}
  end

  defp content(%{rest: ""} = state, kind, _), do: error!(state, "missing closing </#{kind}>")

  defp content(%{rest: "<" <> _} = state, kind, children) do
    if kind in ["text", "button"], do: error!(state, "#{kind} cannot contain child elements")
    {child, state} = element(state)
    content(state, kind, [child | children])
  end

  defp content(%{rest: "{" <> _} = state, kind, children) do
    {expression, state} = expression(state)
    content(state, kind, [expression | children])
  end

  defp content(state, kind, children) do
    length =
      case :binary.match(state.rest, ["<", "{"]) do
        {length, _} -> length
        :nomatch -> byte_size(state.rest)
      end

    text = binary_part(state.rest, 0, length)
    state = consume(state, text)

    children =
      cond do
        kind in ["text", "button"] -> [{:literal, text} | children]
        String.trim(text) == "" -> children
        true -> error!(state, "#{kind} accepts only native child nodes, not raw text")
      end

    content(state, kind, children)
  end

  # Let Elixir's parser identify the matching brace, including braces inside
  # strings, maps, sigils and comments. Wrapping in a one-element tuple ensures
  # an inner closing brace cannot prematurely terminate the expression.
  defp expression(state) do
    initial = consume(state, "{")
    candidates = :binary.matches(initial.rest, "}")

    found =
      Enum.find_value(candidates, fn {length, 1} ->
        text = binary_part(initial.rest, 0, length)

        case Code.string_to_quoted("{" <> text <> "}", file: state.file, line: state.line) do
          {:ok, {:{}, _, [ast]}} -> {ast, length}
          _ -> nil
        end
      end)

    case found do
      {ast, length} ->
        consumed = binary_part(initial.rest, 0, length + 1)
        {{:expression, ast}, consume(initial, consumed)}

      nil ->
        error!(state, "malformed or unterminated brace expression")
    end
  end

  defp name(state) do
    case Regex.run(~r/\A[a-zA-Z_:][a-zA-Z0-9_:\-]*/, state.rest) do
      [name] -> {name, consume(state, name)}
      _ -> error!(state, "expected a tag or attribute name")
    end
  end

  defp whitespace(state) do
    [space] = Regex.run(~r/\A\s*/, state.rest)
    consume(state, space)
  end

  defp consume(state, text) do
    rest = binary_part(state.rest, byte_size(text), byte_size(state.rest) - byte_size(text))
    %{state | rest: rest, line: state.line + length(:binary.matches(text, "\n"))}
  end

  defp emit(node, caller) do
    attrs = node.attrs
    loop = Map.get(attrs, ":for")
    condition = Map.get(attrs, ":if")

    if loop && not Map.has_key?(attrs, "id"),
      do:
        error!(
          %{file: caller.file, line: node.line},
          ":for requires an explicit stable id on the repeated node"
        )

    props =
      attrs
      |> Map.drop([":for", ":if"])
      |> Enum.sort()
      |> Enum.map(fn {key, value} ->
        {key, value_ast(value, caller)}
      end)

    children =
      Enum.map(node.children, fn
        %{kind: _} = child ->
          emit(child, caller)

        value ->
          ast = value_ast(value, caller)

          quote line: node.line do
            LiveGPUI.Template.text!(unquote(ast))
          end
      end)

    ast =
      quote line: node.line do
        LiveGPUI.Node.new!(
          unquote(node.kind),
          unquote({:%{}, [], props}),
          List.flatten(unquote(children))
        )
      end

    ast =
      if loop do
        quote line: node.line do
          LiveGPUI.Template.repeated!(unquote(ast))
        end
      else
        ast
      end

    ast =
      if condition do
        predicate = value_ast(condition, caller)

        quote line: node.line do
          if unquote(predicate), do: unquote(ast), else: []
        end
      else
        ast
      end

    emit_loop(ast, loop, caller, node.line)
  end

  defp emit_loop(ast, nil, _caller, _line), do: ast

  defp emit_loop(ast, {:expression, {:<-, _, [_pattern, _enumerable]} = generator}, caller, line) do
    generator = rewrite_assigns(generator, caller)

    quote line: line do
      for unquote(generator), do: unquote(ast)
    end
  end

  defp emit_loop(_ast, _loop, caller, line) do
    error!(%{file: caller.file, line: line}, ":for must be a generator such as item <- @items")
  end

  defp value_ast({:literal, value}, _caller), do: Macro.escape(value)
  defp value_ast({:expression, ast}, caller), do: rewrite_assigns(ast, caller)

  defp rewrite_assigns(ast, caller) do
    Macro.prewalk(ast, fn
      {:@, meta, [{key, _, context}]} when is_atom(key) and is_atom(context) ->
        assigns = Macro.var(:assigns, nil)

        quote line: Keyword.get(meta, :line, caller.line) do
          Map.fetch!(unquote(assigns), unquote(key))
        end

      other ->
        other
    end)
  end

  defp error!(state, description) do
    raise CompileError, file: state.file, line: state.line, description: description
  end
end
