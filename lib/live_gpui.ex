defmodule LiveGPUI do
  @moduledoc """
  Elixir application state, Rustler messaging, and a main-thread GPUI desktop host.

  `open/2` starts one native window and a restartable `LiveGPUI.View` worker.
  The session retains Elixir checkpoints across worker restarts; it does not
  persist state across application or session termination.
  """

  alias LiveGPUI.Session

  @doc "Opens a native view. Options include `:params`, `:session`, `:name`, and `:host`."
  @spec open(module(), keyword()) :: Supervisor.on_start_child()
  def open(view, options \\ []) do
    DynamicSupervisor.start_child(LiveGPUI.Sessions, {Session, [{:view, view} | options]})
  end

  @doc "Closes the native window and its view worker."
  @spec close(GenServer.server()) :: :ok
  def close(session), do: GenServer.stop(session, :normal)

  @doc "Returns the current view worker PID without waiting for its application callback."
  @spec view_pid(GenServer.server()) :: pid()
  def view_pid(session), do: GenServer.call(session, :view_pid)

  @doc "Restarts the worker, restoring its latest Elixir checkpoint through a new snapshot."
  @spec restart_view(GenServer.server()) :: :ok
  def restart_view(session), do: GenServer.call(session, :restart_view)

  @doc "Inspects actual native entities, text selections, and the applied protocol revision."
  @spec inspect_native(GenServer.server(), timeout()) :: {:ok, map()} | {:error, term()}
  def inspect_native(session, timeout \\ 5000) do
    GenServer.call(session, {:inspect_native, timeout}, timeout + 1000)
  end
end
