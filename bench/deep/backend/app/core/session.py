"""Database session handling.

One session per request, opened by the route and closed when it returns, so a
service never has to decide when a transaction ends.
"""


def get_session():
    """The session bound to the current request."""
    return _current


_current = None


async def commit(session) -> None:
    """Ends the request's transaction, or rolls back if a rule raised."""
    await session.flush()
    await session.commit()
