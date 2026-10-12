defmodule LiveGPUI.Demo do
  @moduledoc "A text editor and keyed-list proof, including a deliberately busy application callback."
  use LiveGPUI.View

  @impl true
  def mount(_params, _session, socket) do
    {:ok,
     assign(socket,
       draft: "",
       draft_revision: 0,
       completed_loads: 0,
       next_id: 4,
       items: [
         %{id: "1", value: "Alpha", edit_revision: 0},
         %{id: "2", value: "Bravo", edit_revision: 0},
         %{id: "3", value: "Charlie", edit_revision: 0}
       ]
     )}
  end

  @impl true
  def render(assigns) do
    ~G"""
    <column id="demo" gap={16}>
      <text id="heading">live_gpui · Native interaction, Elixir application state</text>
      <text id="instructions">Type here while the Elixir callback is busy. Reorder the keyed editors without losing selection.</text>
      <input id="draft" value={@draft} edit_revision={@draft_revision}
        placeholder="Native text composition" phx-change="draft" />
      <row id="actions" gap={12}>
        <button id="busy" phx-click="busy">Busy Elixir for 5 seconds</button>
        <button id="reverse" phx-click="reverse">Reverse keyed list</button>
        <button id="add" phx-click="add">Add row</button>
        <button id="restart" phx-click="restart">Restart view process</button>
      </row>
      <text id="status">Completed load runs: {@completed_loads}. Acknowledged draft revision: {@draft_revision}.</text>
      <list id="items" gap={10}>
        <row :for={item <- @items} id={"item-" <> item.id} gap={12}>
          <text id={"label-" <> item.id}>Key {item.id}</text>
          <input id={"input-" <> item.id} value={item.value} edit_revision={item.edit_revision}
            phx-change="item" phx-value-item-id={item.id} />
          <button id={"remove-" <> item.id} phx-click="remove" phx-value-item-id={item.id}>Remove</button>
        </row>
      </list>
    </column>
    """
  end

  @impl true
  def handle_event("draft", %{"value" => value, "edit_revision" => revision}, socket) do
    {:noreply, assign(socket, draft: value, draft_revision: revision)}
  end

  def handle_event(
        "item",
        %{"item-id" => id, "value" => value, "edit_revision" => revision},
        socket
      ) do
    {:noreply,
     update(socket, :items, fn items ->
       Enum.map(items, fn
         %{id: ^id} = item -> %{item | value: value, edit_revision: revision}
         item -> item
       end)
     end)}
  end

  def handle_event("reverse", _params, socket),
    do: {:noreply, update(socket, :items, &Enum.reverse/1)}

  def handle_event("add", _params, socket) do
    id = Integer.to_string(socket.assigns.next_id)
    item = %{id: id, value: "New keyed editor", edit_revision: 0}
    {:noreply, socket |> update(:items, &(&1 ++ [item])) |> update(:next_id, &(&1 + 1))}
  end

  def handle_event("remove", %{"item-id" => id}, socket) do
    {:noreply, update(socket, :items, &Enum.reject(&1, fn item -> item.id == id end))}
  end

  def handle_event("busy", _params, socket) do
    busy_until(System.monotonic_time(:millisecond) + 5000)
    {:noreply, update(socket, :completed_loads, &(&1 + 1))}
  end

  def handle_event("restart", _params, _socket), do: exit(:demo_restart)

  defp busy_until(deadline) do
    if System.monotonic_time(:millisecond) < deadline, do: busy_until(deadline), else: :ok
  end
end
