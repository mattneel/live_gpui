defmodule LiveGPUI.Node do
  @moduledoc "A validated native node, not an HTML element."
  @enforce_keys [:kind]
  defstruct [:id, :kind, props: %{}, children: []]

  @type t :: %__MODULE__{id: String.t() | nil, kind: String.t(), props: map(), children: [t()]}

  @kinds ~w(column row list text button input)
  @containers ~w(column row list)

  def new!(kind, props, children) when is_map(props) and is_list(children) do
    unless kind in @kinds, do: invalid!("unsupported native kind #{inspect(kind)}")
    {id, props} = Map.pop(props, "id")
    validate_id!(id)
    Enum.each(props, fn {key, value} -> validate_property!(kind, key, value) end)

    {props, children} = normalize_children(kind, props, children)

    %__MODULE__{id: id, kind: kind, props: props, children: children}
  end

  def new!(_, _, _), do: invalid!("properties must be a map and children a list")

  defp normalize_children(kind, props, children) when kind in @containers do
    unless Enum.all?(children, &match?(%__MODULE__{}, &1)),
      do: invalid!("#{kind} children must be native nodes")

    {props, children}
  end

  defp normalize_children("input", props, []), do: {props, []}
  defp normalize_children("input", _props, _children), do: invalid!("input cannot have children")

  defp normalize_children(kind, props, children) do
    unless Enum.all?(children, &is_binary/1), do: invalid!("#{kind} cannot contain child nodes")
    text = children |> Enum.join() |> String.trim()
    {put_text(props, text), []}
  end

  defp put_text(props, ""), do: props

  defp put_text(%{"text" => _}, _text),
    do: invalid!("text must be supplied as either content or a property")

  defp put_text(props, text), do: Map.put(props, "text", text)

  def validate_id!(nil), do: :ok
  def validate_id!(id) when is_binary(id) and byte_size(id) > 0, do: :ok
  def validate_id!(_), do: invalid!("node id must be a nonempty string")

  def allowed_property?(kind, key) do
    case {kind, key} do
      {kind, "gap"} when kind in @containers -> true
      {kind, "text"} when kind in ["text", "button"] -> true
      {"button", "phx-click"} -> true
      {"input", key} when key in ["value", "placeholder", "edit_revision", "phx-change"] -> true
      {kind, "phx-value-" <> suffix} when kind in ["button", "input"] -> suffix != ""
      _ -> false
    end
  end

  defp validate_property!(kind, key, value) do
    unless is_binary(key) and allowed_property?(kind, key),
      do: invalid!("unsupported #{kind} property #{inspect(key)}")

    unless valid_property_value?(key, value),
      do: invalid!("invalid #{kind} property #{inspect(key)}: #{inspect(value)}")
  end

  defp valid_property_value?("gap", value), do: is_integer(value) and value in 0..128
  defp valid_property_value?("edit_revision", value), do: is_integer(value) and value >= 0

  defp valid_property_value?("phx-value-" <> _, value),
    do: is_binary(value) or is_number(value) or is_boolean(value) or is_nil(value)

  defp valid_property_value?(_key, value), do: is_binary(value)

  defp invalid!(message), do: raise(ArgumentError, message)
end
