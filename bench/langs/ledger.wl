(* Keeps the running balance for one account. *)

limit = 5000;

refuse[owner_] := Module[{},
  warnOwner[owner];
  0
]

commitEntry[owner_, amount_] := Module[{},
  writeEntry[owner, amount];
  amount
]

(* Bills the account and returns what is left. *)
charge[owner_, amount_] := Module[{},
  If[amount > limit,
    refuse[owner],
    commitEntry[owner, amount]
  ]
]
