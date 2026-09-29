"""Stored shape of a order."""


class Order:
    """One order row as it lives in the database."""

    def __init__(self, **fields) -> None:
        self.fields = fields
        self.archived = False
        self.cancelled = False
        self.lines = []
