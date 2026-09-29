defmodule Bench.Ledger do
  @moduledoc "Keeps the running balance for one account."

  @limit 5000

  @doc "Bills the account and returns what is left."
  def charge(owner, amount) when amount > @limit do
    refuse(owner, amount)
  end

  def charge(owner, amount) do
    commit_entry(owner, amount)
  end

  defp refuse(owner, _amount) do
    warn(owner)
    0
  end

  defp commit_entry(owner, amount) do
    write_entry(owner, amount)
    amount
  end
end
