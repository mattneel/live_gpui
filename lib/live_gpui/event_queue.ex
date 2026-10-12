defmodule LiveGPUI.EventQueue do
  @moduledoc false
  defstruct queue: :queue.new(), size: 0, limit: 64

  def push(state, event) do
    case :queue.out_r(state.queue) do
      {{:value, previous}, rest} -> enqueue(state, event, previous, rest)
      {:empty, _} -> append(state, event)
    end
  end

  def pop(state) do
    case :queue.out(state.queue) do
      {{:value, event}, rest} -> {event, %{state | queue: rest, size: state.size - 1}}
      {:empty, _} -> {nil, state}
    end
  end

  defp enqueue(
         state,
         %{"node" => id, "params" => %{"edit_revision" => revision}} = event,
         %{"node" => id, "params" => %{"edit_revision" => previous}},
         rest
       )
       when revision >= previous do
    {:ok, %{state | queue: :queue.in(event, rest)}}
  end

  defp enqueue(state, event, _previous, _rest), do: append(state, event)

  defp append(%{size: size, limit: limit}, _event) when size >= limit,
    do: {:error, :event_overflow}

  defp append(state, event),
    do: {:ok, %{state | queue: :queue.in(event, state.queue), size: state.size + 1}}
end
