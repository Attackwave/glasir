// Keeps the running balance for one account.
module ledger;

  parameter LIMIT = 5000;

  task refuse(input [63:0] owner);
    begin
      warn_owner(owner);
    end
  endtask

  task commit_entry(input [63:0] owner, input [63:0] amount);
    begin
      write_entry(owner, amount);
    end
  endtask

  // Bills the account and returns what is left.
  task charge(input [63:0] owner, input [63:0] amount);
    begin
      if (amount > LIMIT)
        refuse(owner);
      else
        commit_entry(owner, amount);
    end
  endtask

endmodule
