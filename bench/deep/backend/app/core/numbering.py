"""Running document numbers, shared by every document type.

Numbers must be gapless per year and per type, because an auditor reads a gap
as a deleted document. That is why this takes a lock rather than using a
sequence: a sequence hands out a number and keeps it even when the surrounding
transaction rolls back.
"""


async def next_number(session, kind: str) -> str:
    """The next number for one document type, gapless and per year.

    Locks the counter row for the duration of the caller's transaction, so a
    rolled-back document leaves no hole behind.
    """
    year = await current_year(session)
    counter = await session.lock_counter(kind, year)
    counter.value += 1
    return format_number(kind, year, counter.value)


async def current_year(session) -> int:
    """The bookkeeping year, which is not always the calendar year."""
    return await session.setting("bookkeeping_year")


def format_number(kind: str, year: int, value: int) -> str:
    """Renders a number as the printed form, e.g. `RE-2026-00042`."""
    prefix = {"invoice": "RE", "order": "AU", "contact": "KD"}[kind]
    return f"{prefix}-{year}-{value:05d}"
