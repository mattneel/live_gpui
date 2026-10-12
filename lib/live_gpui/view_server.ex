defmodule LiveGPUI.ViewServer do
  @moduledoc false
  use GenServer, restart: :temporary

  alias LiveGPUI.Tree
  alias Phoenix.Component
  alias Phoenix.LiveView.Socket

  defstruct [:module, :socket, :owner, :monitor]

  def start_link(options), do: GenServer.start_link(__MODULE__, options)

  @impl true
  def init(options) do
    owner = Keyword.fetch!(options, :owner)
    module = Keyword.fetch!(options, :view)
    socket = %Socket{assigns: %{__changed__: %{}}, private: %{live_gpui_session: owner}}
    {:ok, socket} = module.mount(options[:params] || %{}, options[:session] || %{}, socket)
    socket = Component.assign(socket, options[:checkpoint] || %{})

    {:ok,
     %__MODULE__{module: module, socket: socket, owner: owner, monitor: Process.monitor(owner)},
     {:continue, :render}}
  end

  @impl true
  def handle_continue(:render, state), do: {:noreply, commit(state, nil)}

  @impl true
  def handle_info({:native_event, reference, event}, state) do
    {:noreply, socket} = state.module.handle_event(event["name"], event["params"], state.socket)
    {:noreply, commit(%{state | socket: socket}, reference)}
  end

  def handle_info({:DOWN, monitor, :process, _owner, _reason}, %{monitor: monitor} = state) do
    {:stop, :normal, state}
  end

  def handle_info(message, state) do
    handle_application_message(message, state, function_exported?(state.module, :handle_info, 2))
  end

  defp handle_application_message(message, state, true) do
    {:noreply, socket} = state.module.handle_info(message, state.socket)
    {:noreply, commit(%{state | socket: socket}, nil)}
  end

  defp handle_application_message(_message, state, false), do: {:noreply, state}

  defp commit(state, event_reference) do
    tree = state.module.render(state.socket.assigns) |> Tree.flatten()
    checkpoint = Map.delete(state.socket.assigns, :__changed__)
    :ok = GenServer.call(state.owner, {:render, self(), event_reference, checkpoint, tree})
    %{state | socket: %{state.socket | assigns: Map.put(checkpoint, :__changed__, %{})}}
  end
end
