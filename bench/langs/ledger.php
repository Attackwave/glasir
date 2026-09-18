<?php

namespace Bench;

/** Keeps the running balance for one account. */
class Ledger
{
    const LIMIT = 5000;

    private $owner;

    public function __construct($owner)
    {
        $this->owner = $owner;
    }

    /** Bills the account and returns what is left. */
    public function charge($amount)
    {
        if ($amount > self::LIMIT) {
            return $this->refuse($amount);
        }
        return $this->commit($amount);
    }

    private function refuse($amount)
    {
        warn($this->owner);
        return 0;
    }

    private function commit($amount)
    {
        writeEntry($this->owner, $amount);
        return $amount;
    }
}
