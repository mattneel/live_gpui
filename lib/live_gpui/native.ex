defmodule LiveGPUI.Native do
  @moduledoc false

  for resource <-
        ["Cargo.toml", "Cargo.lock", "native/vendor/zed-xim/Cargo.toml"] ++
          Path.wildcard("native/vendor/zed-xim/src/**/*.{rs,txt}") do
    @external_resource resource
  end

  use Rustler,
    otp_app: :live_gpui,
    crate: :live_gpui_host,
    lib: false,
    binaries: ["live_gpui_host"],
    mode: :debug

  use Rustler, otp_app: :live_gpui, crate: :live_gpui_native, mode: :debug

  def open(_executable, _owner, _resource_id), do: :erlang.nif_error(:nif_not_loaded)
  def submit(_resource, _message), do: :erlang.nif_error(:nif_not_loaded)
  def close(_resource), do: :erlang.nif_error(:nif_not_loaded)

  def host_path do
    suffix = if match?({:win32, _}, :os.type()), do: ".exe", else: ""
    Application.app_dir(:live_gpui, "priv/native/live_gpui_host" <> suffix)
  end
end
