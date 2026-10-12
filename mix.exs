defmodule LiveGPUI.MixProject do
  use Mix.Project

  def project do
    [
      app: :live_gpui,
      version: "0.1.0",
      elixir: "~> 1.20",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      dialyzer: [plt_add_apps: [:ex_unit]],
      aliases: aliases(),
      usage_rules: [file: "AGENTS.md", usage_rules: :all]
    ]
  end

  def application do
    [
      mod: {LiveGPUI.Application, []},
      extra_applications: [:logger, :crypto]
    ]
  end

  def cli do
    [
      preferred_envs: [ci: :test]
    ]
  end

  defp deps do
    [
      {:usage_rules, "~> 1.0", only: [:dev]},
      {:ex_slop, "~> 0.4", only: [:dev, :test], runtime: false},
      {:reach, "~> 2.0", only: [:dev, :test], runtime: false},
      {:ex_dna, "~> 1.0", only: [:dev, :test], runtime: false},
      {:dialyxir, "~> 1.0", only: [:dev, :test], runtime: false},
      {:credo, "~> 1.0", only: [:dev, :test], runtime: false},
      {:phoenix_live_view, "~> 1.0"},
      {:rustler, "~> 0.38"},
      {:jason, "~> 1.4"},
      {:vibe_kit, "~> 0.1"},
      {:igniter, "~> 0.6", only: [:dev, :test]}
    ]
  end

  defp aliases() do
    [
      ci: [
        "compile --warnings-as-errors",
        "format --check-formatted",
        "test",
        "credo --strict",
        "dialyzer",
        "ex_dna --max-clones 0",
        "reach.check --arch --smells"
      ]
    ]
  end
end
