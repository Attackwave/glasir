''' An attribute in angle brackets is a declaration, not a call: a generated
''' AssemblyInfo.vb is nothing but these.
<Assembly: AssemblyTitle("Ledger")>

Namespace Bench

    ' Keeps the running balance for one account.
    Public Class Ledger

        Private Const LIMIT As Integer = 5000

        Private ReadOnly owner As String

        Public Sub New(account As String)
            owner = account
        End Sub

        ' Bills the account and returns what is left.
        Public Function Charge(amount As Integer) As Integer
            If amount > LIMIT Then
                Return Refuse(amount)
            End If
            Return CommitEntry(amount)
        End Function

        Private Function Refuse(amount As Integer) As Integer
            WarnOwner(owner)
            Return 0
        End Function

        Private Function CommitEntry(amount As Integer) As Integer
            WriteEntry(owner, amount)
            Return amount
        End Function

    End Class

End Namespace
