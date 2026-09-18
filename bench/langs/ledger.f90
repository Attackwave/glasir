! Keeps the running balance for one account.
module ledger
   implicit none
   integer, parameter :: LIMIT = 5000

contains

   function refuse(owner) result(left)
      character(*), intent(in) :: owner
      integer :: left
      call warn_owner(owner)
      left = 0
   end function refuse

   function commit_entry(owner, amount) result(left)
      character(*), intent(in) :: owner
      integer, intent(in) :: amount
      integer :: left
      call write_entry(owner, amount)
      left = amount
   end function commit_entry

   ! Bills the account and returns what is left.
   function charge(owner, amount) result(left)
      character(*), intent(in) :: owner
      integer, intent(in) :: amount
      integer :: left
      if (amount > LIMIT) then
         left = refuse(owner)
         return
      end if
      left = commit_entry(owner, amount)
   end function charge

end module ledger
