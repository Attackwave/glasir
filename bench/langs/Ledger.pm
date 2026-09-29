package Bench::Ledger;

# Keeps the running balance for one account.

use strict;
use warnings;

use constant LIMIT => 5000;

# Bills the account and returns what is left.
sub charge {
    my ($owner, $amount) = @_;
    if ($amount > LIMIT) {
        return refuse($owner, $amount);
    }
    return commit_entry($owner, $amount);
}

sub refuse {
    my ($owner, $amount) = @_;
    warn_owner($owner);
    return 0;
}

sub commit_entry {
    my ($owner, $amount) = @_;
    write_entry($owner, $amount);
    return $amount;
}

1;
