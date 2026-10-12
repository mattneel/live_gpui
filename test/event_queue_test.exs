defmodule LiveGPUI.EventQueueTest do
  use ExUnit.Case, async: true
  alias LiveGPUI.EventQueue

  test "adjacent edits coalesce to the latest complete value while the view is busy" do
    {:ok, queue} = EventQueue.push(%EventQueue{limit: 1}, edit("draft", 1, "a"))
    {:ok, queue} = EventQueue.push(queue, edit("draft", 2, "ab"))
    {event, queue} = EventQueue.pop(queue)
    assert event["params"] == %{"edit_revision" => 2, "value" => "ab"}
    assert {nil, ^queue} = EventQueue.pop(queue)
  end

  test "edits do not coalesce across a semantic action boundary" do
    add = %{"node" => "add", "name" => "add", "params" => %{}}
    {:ok, queue} = EventQueue.push(%EventQueue{}, edit("draft", 1, "before"))
    {:ok, queue} = EventQueue.push(queue, add)
    {:ok, queue} = EventQueue.push(queue, edit("draft", 2, "after"))
    {first, queue} = EventQueue.pop(queue)
    {action, queue} = EventQueue.pop(queue)
    {last, _queue} = EventQueue.pop(queue)
    assert first["params"]["value"] == "before"
    assert action == add
    assert last["params"]["value"] == "after"
  end

  test "a full discrete queue fails explicitly instead of silently dropping an action" do
    event = %{"node" => "add", "name" => "add", "params" => %{}}
    {:ok, queue} = EventQueue.push(%EventQueue{limit: 1}, event)
    assert {:error, :event_overflow} = EventQueue.push(queue, event)
    assert {^event, _} = EventQueue.pop(queue)
  end

  defp edit(id, revision, value) do
    %{
      "node" => id,
      "name" => "change",
      "params" => %{"edit_revision" => revision, "value" => value}
    }
  end
end
