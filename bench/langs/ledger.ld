/* Keeps the running balance for one account. */

SECTIONS
{
    .charge : {
        *(.refuse)
        *(.commit_entry)
    }

    .refuse : {
        *(.text.warn_owner)
    }

    .commit_entry : {
        *(.text.write_entry)
    }
}
