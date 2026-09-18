"""The two failures every layer above shares.

Deliberately two and not a hierarchy: a caller only ever needs to know whether
the thing was there and whether the request made sense.
"""


class NotFound(Exception):
    """The addressed row does not exist. Becomes a 404 at the edge."""


class Invalid(Exception):
    """The request contradicts a business rule. Becomes a 422 at the edge."""
