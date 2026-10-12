defmodule LiveGPUI.TreeTest do
  use ExUnit.Case, async: true
  alias LiveGPUI.{Node, Tree}

  test "reordering keyed controls only patches the parent's child order" do
    a = Node.new!("input", %{"id" => "a", "value" => "alpha"}, [])
    b = Node.new!("input", %{"id" => "b", "value" => "bravo"}, [])
    before = Node.new!("list", %{"id" => "list"}, [a, b]) |> Tree.flatten()
    after_tree = Node.new!("list", %{"id" => "list"}, [b, a]) |> Tree.flatten()
    patch = Tree.patch(before, after_tree, 7, 12)

    assert patch["generation"] == 7
    assert patch["base_revision"] == 12
    assert patch["revision"] == 13
    assert patch["remove"] == []
    assert [%{"id" => "list", "children" => ["b", "a"]}] = patch["upsert"]
  end

  test "static descendants are scoped to their keyed parent, not list position" do
    row = fn id -> Node.new!("row", %{"id" => id}, [Node.new!("input", %{"value" => id}, [])]) end
    before = Node.new!("list", %{}, [row.("a"), row.("b")]) |> Tree.flatten()
    next = Node.new!("list", %{}, [row.("b"), row.("a")]) |> Tree.flatten()
    assert before["nodes"]["a/0"] == next["nodes"]["a/0"]
    assert next["nodes"]["a"]["children"] == ["a/0"]
    assert next["nodes"]["b"]["children"] == ["b/0"]
  end

  test "removal and replacement produce explicit patch operations" do
    before =
      Node.new!("column", %{}, [Node.new!("text", %{"id" => "gone"}, ["old"])]) |> Tree.flatten()

    next =
      Node.new!("column", %{}, [Node.new!("input", %{"id" => "new", "value" => "new"}, [])])
      |> Tree.flatten()

    patch = Tree.patch(before, next, 1, 1)
    assert patch["remove"] == ["gone"]
    assert Enum.any?(patch["upsert"], &(&1["id"] == "new" and &1["kind"] == "input"))
    assert Enum.any?(patch["upsert"], &(&1["id"] == "root" and &1["children"] == ["new"]))
  end

  test "duplicate explicit and generated IDs fail before the NIF boundary" do
    child = Node.new!("text", %{"id" => "root"}, ["collision"])
    assert_raise ArgumentError, fn -> Node.new!("column", %{}, [child]) |> Tree.flatten() end
  end

  test "tree depth is bounded before crossing into native code" do
    leaf = Node.new!("text", %{}, ["deep"])
    root = Enum.reduce(1..64, leaf, fn _, child -> Node.new!("column", %{}, [child]) end)
    assert_raise ArgumentError, fn -> Tree.flatten(root) end
  end
end
