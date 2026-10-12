defmodule LiveGPUI.View do
  @moduledoc """
  Callback-compatible native views using a Phoenix socket and native `~G` templates.

  This is not a Phoenix channel, HEEx/HTML renderer, or LiveView diff adapter.
  Application callbacks run in an isolated, restartable Elixir process.
  """

  alias LiveGPUI.Node
  alias Phoenix.LiveView.Socket

  @callback mount(map(), map(), Socket.t()) :: {:ok, Socket.t()}
  @callback render(map()) :: Node.t()
  @callback handle_event(String.t(), map(), Socket.t()) :: {:noreply, Socket.t()}
  @callback handle_info(term(), Socket.t()) :: {:noreply, Socket.t()}
  @optional_callbacks handle_info: 2

  defmacro __using__(_options) do
    quote do
      @behaviour LiveGPUI.View
      import Phoenix.Component, only: [assign: 2, assign: 3, update: 3]
      import LiveGPUI.View, only: [sigil_G: 2]
    end
  end

  @doc "Compiles the explicitly supported native template language."
  defmacro sigil_G({:<<>>, _meta, [source]}, []) when is_binary(source) do
    LiveGPUI.Template.compile(source, __CALLER__)
  end
end
