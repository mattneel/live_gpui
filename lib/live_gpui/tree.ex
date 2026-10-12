defmodule LiveGPUI.Tree do
  @moduledoc "Stable-ID native trees and application-level snapshot/patch construction."

  alias LiveGPUI.Node

  @max_nodes 4096
  @max_depth 64
  @type t :: %{String.t() => term()}

  @doc "Flattens a native declaration, rejecting duplicate IDs and oversized trees."
  @spec flatten(Node.t()) :: t()
  def flatten(%Node{} = root) do
    {id, nodes} = flatten_node(root, "root", %{}, 1)
    %{"root" => id, "nodes" => nodes}
  end

  @doc "Builds a full recovery snapshot for one generation and revision."
  @spec snapshot(t(), pos_integer(), pos_integer()) :: map()
  def snapshot(tree, generation, revision) do
    %{"type" => "snapshot", "generation" => generation, "revision" => revision, "tree" => tree}
  end

  @doc "Diffs from the last acknowledged tree, retaining unchanged keyed controls."
  @spec patch(t(), t(), pos_integer(), pos_integer()) :: map()
  def patch(previous, next, generation, base_revision) do
    upsert =
      next["nodes"]
      |> Enum.reject(fn {id, node} -> previous["nodes"][id] == node end)
      |> Enum.sort_by(&elem(&1, 0))
      |> Enum.map(&elem(&1, 1))

    remove = previous["nodes"] |> Map.keys() |> Enum.reject(&Map.has_key?(next["nodes"], &1))

    %{
      "type" => "patch",
      "generation" => generation,
      "base_revision" => base_revision,
      "revision" => base_revision + 1,
      "root" => next["root"],
      "upsert" => upsert,
      "remove" => Enum.sort(remove)
    }
  end

  defp flatten_node(_node, _fallback, nodes, depth)
       when map_size(nodes) >= @max_nodes or depth > @max_depth do
    raise ArgumentError, "native tree exceeds #{@max_nodes} nodes or #{@max_depth} levels"
  end

  defp flatten_node(%Node{} = node, fallback, nodes, depth) do
    id = node.id || fallback

    if Map.has_key?(nodes, id) do
      raise ArgumentError, "duplicate native node ID: #{inspect(id)}"
    end

    nodes = Map.put(nodes, id, nil)

    {children, nodes} =
      node.children
      |> Enum.with_index()
      |> Enum.map_reduce(nodes, fn {child, index}, acc ->
        flatten_node(child, id <> "/" <> Integer.to_string(index), acc, depth + 1)
      end)

    flat = %{"id" => id, "kind" => node.kind, "props" => node.props, "children" => children}
    {id, Map.put(nodes, id, flat)}
  end
end
