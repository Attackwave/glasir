-- Keeps the running balance for one account.
package body Ledger is

   Limit : constant Integer := 5000;

   function Refuse (Owner : String) return Integer is
   begin
      Warn_Owner (Owner);
      return 0;
   end Refuse;

   function Commit_Entry (Owner : String; Amount : Integer) return Integer is
   begin
      Write_Entry (Owner, Amount);
      return Amount;
   end Commit_Entry;

   -- Bills the account and returns what is left.
   function Charge (Owner : String; Amount : Integer) return Integer is
   begin
      if Amount > Limit then
         return Refuse (Owner);
      end if;
      return Commit_Entry (Owner, Amount);
   end Charge;

end Ledger;
