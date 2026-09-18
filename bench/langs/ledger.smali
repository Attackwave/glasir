# Keeps the running balance for one account.
.class public Lcom/example/Ledger;
.super Ljava/lang/Object;

.field private static final LIMIT:I = 0x1388

# Bills the account and returns what is left.
.method public charge(I)I
    invoke-direct {p0}, Lcom/example/Ledger;->refuse(I)I
    invoke-direct {p0}, Lcom/example/Ledger;->commitEntry(I)I
    return p1
.end method

.method private refuse(I)I
    invoke-static {p1}, Lcom/example/Ledger;->warnOwner(I)V
    return p1
.end method

.method private commitEntry(I)I
    invoke-static {p1}, Lcom/example/Ledger;->writeEntry(I)V
    return p1
.end method
