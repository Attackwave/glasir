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
    Enum.each([owner], fn name -> notify(name) end)
    if overdrawn?(owner) in @limit do
      close!(owner)
    end
    0
  end

  defp overdrawn?(owner), do: lookup(owner)

  defp close!(owner) do
    ~S|end of the account do|
    archive(owner)
  end

  defp commit_entry(owner, amount) do
    write_entry(owner, amount)
    amount
  end
end
