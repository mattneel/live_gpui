defmodule LiveGPUI.Application do
  @moduledoc false
  use Application

  @impl true
  def start(_type, _arguments) do
    children = [
      {DynamicSupervisor, name: LiveGPUI.Views, strategy: :one_for_one},
      {DynamicSupervisor, name: LiveGPUI.Sessions, strategy: :one_for_one}
    ]

    Supervisor.start_link(children, strategy: :one_for_one, name: LiveGPUI.Supervisor)
  end
end
