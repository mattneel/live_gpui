defmodule LiveGPUI.TemplateTest.Fixture do
  import LiveGPUI.View

  def counter(assigns) do
    ~G"""
    <column gap={@gap}>
      <text>Count: {@count}</text>
      <button phx-click="increment" phx-value-step={@step}>Add {@step}</button>
      <input id="entry" value={@value} edit_revision={@revision} placeholder="Type here" phx-change="edit" />
    </column>
    """
  end

  def groups(assigns) do
    ~G"""
    <list id="groups">
      <column :for={group <- @groups} id={group.id}>
        <text id={group.id <> "-label"}>{group.label}</text>
        <row :for={item <- group.items} :if={item.visible} id={item.id}>
          <text>{item.name}</text>
        </row>
      </column>
    </list>
    """
  end

  def expressions(assigns) do
    ~G"""
    <text>{Map.fetch!(%{"}" => @value}, "}")} {String.upcase("{ok}")}</text>
    """
  end
end

defmodule LiveGPUI.TemplateTest do
  use ExUnit.Case, async: true
  alias LiveGPUI.{Node, Template}
  alias LiveGPUI.TemplateTest.Fixture

  test "counter renders native data with dynamic assigns, text, and event properties" do
    tree = Fixture.counter(%{gap: 8, count: 12, step: 2, value: "hello", revision: 4})
    assert %Node{id: "root", kind: "column", props: %{"gap" => 8}} = tree
    assert [text, button, input] = tree.children
    assert text.kind == "text"
    assert text.props == %{"text" => "Count: 12"}
    assert text.children == []
    assert button.props == %{"text" => "Add 2", "phx-click" => "increment", "phx-value-step" => 2}
    assert input.id == "entry"

    assert input.props == %{
             "value" => "hello",
             "edit_revision" => 4,
             "placeholder" => "Type here",
             "phx-change" => "edit"
           }

    assert input.children == []
  end

  test "nested keyed loops flatten children, filter conditionally, and retain ids after reorder" do
    first = %{
      id: "group-a",
      label: "A",
      items: [
        %{id: "a-one", name: "one", visible: true},
        %{id: "a-hidden", name: "hidden", visible: false},
        %{id: "a-two", name: "two", visible: true}
      ]
    }

    second = %{id: "group-b", label: "B", items: [%{id: "b-one", name: "three", visible: true}]}
    tree = Fixture.groups(%{groups: [first, second]})
    assert Enum.map(tree.children, & &1.id) == ["group-a", "group-b"]
    assert Enum.map(hd(tree.children).children, & &1.id) == ["group-a-label", "a-one", "a-two"]
    assert hd(Enum.at(hd(tree.children).children, 1).children).props["text"] == "one"

    reordered = Fixture.groups(%{groups: [second, %{first | items: Enum.reverse(first.items)}]})
    assert Enum.map(reordered.children, & &1.id) == ["group-b", "group-a"]

    assert Enum.map(Enum.at(reordered.children, 1).children, & &1.id) == [
             "group-a-label",
             "a-two",
             "a-one"
           ]

    assert Fixture.groups(%{groups: []}).children == []
  end

  test "Elixir expressions preserve braces in maps and strings" do
    assert Fixture.expressions(%{value: "native"}).props["text"] == "native {OK}"
  end

  test "missing assigns fail rather than silently rendering" do
    assert_raise KeyError, fn -> Fixture.counter(%{}) end
  end

  test "a repeated node cannot fall back to an unkeyed identity" do
    group = %{
      id: "group",
      label: "valid parent",
      items: [%{id: nil, name: "item", visible: true}]
    }

    assert_raise ArgumentError, fn -> Fixture.groups(%{groups: [group]}) end
  end

  test "unsupported HTML and attributes fail with caller provenance" do
    env = %{__ENV__ | file: "native_template_fixture.ex", line: 73}

    for source <- ["<div />", "<column class=\"web\" />", "<button onclick=\"web()\" />"] do
      error = assert_raise CompileError, fn -> Template.compile(source, env) end
      assert error.file == "native_template_fixture.ex"
      assert error.line == 73
    end
  end

  test "malformed templates and unkeyed loops are compile errors" do
    for source <- [
          "",
          "<text>unclosed",
          "<row></column>",
          "<text><row /></text>",
          "<input></input>",
          "<text id=plain />",
          "<text>{1 +}</text>",
          "<column><text :for={item <- @items}>{item}</text></column>",
          "<column><text id=\"x\" :for={@items}>x</text></column>",
          "<text id=\"x\" id=\"y\" />",
          "<text id=\"x\"text=\"bad\" />",
          "<text /><text />"
        ] do
      assert_raise CompileError, fn -> Template.compile(source, __ENV__) end
    end
  end

  test "nodes normalize text and validate native property and child invariants" do
    assert Node.new!("button", %{"id" => "go", "phx-click" => "go"}, ["  Go ", "now  "]).props[
             "text"
           ] == "Go now"

    assert Node.new!(
             "input",
             %{"value" => "", "edit_revision" => 0, "phx-value-flag" => false},
             []
           ).kind == "input"

    assert Node.new!("column", %{"gap" => 128}, []).id == nil

    for {kind, props, children} <- [
          {"div", %{}, []},
          {"column", %{"gap" => -1}, []},
          {"row", %{"gap" => 129}, []},
          {"column", %{}, ["raw text"]},
          {"text", %{"text" => 5}, []},
          {"text", %{}, [Node.new!("row", %{}, [])]},
          {"text", %{"text" => "property"}, ["content"]},
          {"input", %{}, ["child"]},
          {"input", %{"edit_revision" => -1}, []},
          {"button", %{"phx-click" => :event}, []},
          {"button", %{"phx-value-map" => %{}}, []},
          {"text", %{"id" => ""}, []},
          {"text", %{text: "atom key"}, []}
        ] do
      assert_raise ArgumentError, fn -> Node.new!(kind, props, children) end
    end
  end
end
