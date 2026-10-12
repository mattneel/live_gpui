defmodule LiveGPUI.Session do
  @moduledoc false
  use GenServer, restart: :transient

  alias LiveGPUI.{EventQueue, Native, Tree, ViewServer}

  defstruct [
    :options,
    :native,
    :resource_id,
    :worker,
    :monitor,
    :desired,
    :applied,
    :flight,
    :event,
    :retry,
    generation: 1,
    revision: 0,
    ready: false,
    checkpoint: %{},
    events: %EventQueue{},
    inspections: %{},
    restart_times: []
  ]

  def start_link(options) do
    GenServer.start_link(__MODULE__, options, Keyword.take(options, [:name]))
  end

  @impl true
  def init(options) do
    Process.flag(:trap_exit, true)
    id = System.unique_integer([:positive])

    with {:ok, native} <- Native.open(options[:host] || Native.host_path(), self(), id),
         {:ok, state} <-
           start_worker(%__MODULE__{options: options, native: native, resource_id: id}) do
      {:ok, state}
    else
      {:error, reason} -> {:stop, reason}
    end
  end

  @impl true
  def handle_call(:view_pid, _from, state), do: {:reply, state.worker, state}

  def handle_call(:restart_view, _from, state) do
    Process.exit(state.worker, :kill)
    {:reply, :ok, state}
  end

  def handle_call(
        {:render, worker, reference, checkpoint, tree},
        _from,
        %{worker: worker} = state
      ) do
    state = %{state | checkpoint: checkpoint, desired: tree}
    state = if reference == state.event, do: %{state | event: nil}, else: state
    {:reply, :ok, state |> publish() |> dispatch_event()}
  end

  def handle_call({:render, _worker, _reference, _checkpoint, _tree}, _from, state) do
    {:reply, :ok, state}
  end

  def handle_call({:inspect_native, timeout}, from, state) do
    request = System.unique_integer([:positive])

    case submit(state, %{"type" => "inspect", "request" => request}) do
      :ok ->
        timer = Process.send_after(self(), {:inspection_timeout, request}, timeout)
        {:noreply, %{state | inspections: Map.put(state.inspections, request, {from, timer})}}

      error ->
        {:reply, error, state}
    end
  end

  @impl true
  def handle_info({:live_gpui_native, id, payload}, %{resource_id: id} = state) do
    case Jason.decode(payload) do
      {:ok, message} -> native_message(message, state)
      {:error, reason} -> {:stop, {:invalid_native_message, reason}, state}
    end
  end

  def handle_info(
        {:DOWN, monitor, :process, worker, _reason},
        %{monitor: monitor, worker: worker} = state
      ) do
    restart_worker(state)
  end

  def handle_info(:publish, state), do: {:noreply, publish(%{state | retry: nil})}

  def handle_info({:inspection_timeout, request}, state) do
    {waiting, inspections} = Map.pop(state.inspections, request)
    if waiting, do: GenServer.reply(elem(waiting, 0), {:error, :timeout})
    {:noreply, %{state | inspections: inspections}}
  end

  def handle_info({:publish_timeout, generation, revision}, state) do
    recover_timed_out_publish(state, generation, revision)
  end

  def handle_info(_message, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
    if state.worker, do: DynamicSupervisor.terminate_child(LiveGPUI.Views, state.worker)
    if state.native, do: Native.close(state.native)

    Enum.each(state.inspections, fn {_request, {from, _timer}} ->
      GenServer.reply(from, {:error, :closed})
    end)
  end

  defp native_message(%{"type" => "ready"}, state),
    do: {:noreply, publish(%{state | ready: true})}

  defp native_message(%{"type" => "closed"}, state), do: {:stop, :normal, state}

  defp native_message(%{"type" => "inspection", "request" => request} = message, state) do
    {waiting, inspections} = Map.pop(state.inspections, request)

    if waiting do
      {from, timer} = waiting
      Process.cancel_timer(timer)
      GenServer.reply(from, {:ok, message})
    end

    {:noreply, %{state | inspections: inspections}}
  end

  defp native_message(
         %{"type" => "applied", "generation" => generation, "revision" => revision},
         %{generation: generation, flight: %{revision: revision} = flight} = state
       ) do
    Process.cancel_timer(flight.timer)
    {:noreply, publish(%{state | applied: flight.tree, revision: revision, flight: nil})}
  end

  defp native_message(
         %{"type" => "resync", "generation" => generation, "revision" => revision},
         %{generation: generation} = state
       ) do
    cancel_flight(state.flight)

    {:noreply,
     publish(%{state | applied: nil, flight: nil, revision: max(state.revision, revision)})}
  end

  defp native_message(
         %{"type" => "event", "generation" => generation} = event,
         %{generation: generation} = state
       ) do
    case EventQueue.push(state.events, event) do
      {:ok, events} -> {:noreply, dispatch_event(%{state | events: events})}
      {:error, reason} -> {:stop, {:shutdown, reason}, state}
    end
  end

  defp native_message(_message, state), do: {:noreply, state}

  defp start_worker(state) do
    options = state.options ++ [owner: self(), checkpoint: state.checkpoint]

    case DynamicSupervisor.start_child(LiveGPUI.Views, {ViewServer, options}) do
      {:ok, worker} -> {:ok, %{state | worker: worker, monitor: Process.monitor(worker)}}
      error -> error
    end
  end

  defp restart_worker(state) do
    now = System.monotonic_time(:millisecond)
    recent = Enum.filter(state.restart_times, &(now - &1 < 5000))
    restart_worker(state, now, recent)
  end

  defp restart_worker(state, _now, [_, _, _ | _]) do
    {:stop, {:shutdown, :view_restart_intensity}, state}
  end

  defp restart_worker(state, now, recent) do
    cancel_flight(state.flight)

    state = %{
      state
      | generation: state.generation + 1,
        revision: 0,
        desired: nil,
        applied: nil,
        flight: nil,
        event: nil,
        events: %EventQueue{},
        restart_times: [now | recent]
    }

    case start_worker(state) do
      {:ok, state} -> {:noreply, state}
      {:error, reason} -> {:stop, reason, state}
    end
  end

  defp dispatch_event(%{event: reference} = state) when not is_nil(reference), do: state
  defp dispatch_event(%{desired: nil} = state), do: state

  defp dispatch_event(state) do
    {event, events} = EventQueue.pop(state.events)
    dispatch_event(event, %{state | events: events})
  end

  defp dispatch_event(nil, state), do: state

  defp dispatch_event(event, state) do
    reference = make_ref()
    send(state.worker, {:native_event, reference, event})
    %{state | event: reference}
  end

  defp publish(%{ready: false} = state), do: state
  defp publish(%{desired: nil} = state), do: state
  defp publish(%{flight: flight} = state) when not is_nil(flight), do: state
  defp publish(%{desired: tree, applied: tree} = state), do: state

  defp publish(state) do
    command = update_command(state)

    case submit(state, command) do
      :ok ->
        revision = command["revision"]
        timer = Process.send_after(self(), {:publish_timeout, state.generation, revision}, 5000)
        %{state | flight: %{tree: state.desired, revision: revision, timer: timer}}

      {:error, :busy} ->
        schedule_snapshot(state)

      {:error, reason} ->
        exit({:native_update_rejected, reason})
    end
  end

  defp update_command(%{applied: nil} = state) do
    Tree.snapshot(state.desired, state.generation, state.revision + 1)
  end

  defp update_command(state) do
    Tree.patch(state.applied, state.desired, state.generation, state.revision)
  end

  defp schedule_snapshot(%{retry: nil} = state) do
    %{state | applied: nil, retry: Process.send_after(self(), :publish, 10)}
  end

  defp schedule_snapshot(state), do: %{state | applied: nil}

  defp recover_timed_out_publish(
         %{generation: generation, flight: %{revision: revision}} = state,
         generation,
         revision
       ) do
    {:noreply, publish(%{state | applied: nil, flight: nil, revision: revision})}
  end

  defp recover_timed_out_publish(state, _generation, _revision), do: {:noreply, state}
  defp cancel_flight(nil), do: :ok
  defp cancel_flight(flight), do: Process.cancel_timer(flight.timer)
  defp submit(state, command), do: Native.submit(state.native, Jason.encode!(command))
end
