"""Stored shape of a contact."""


class Contact:
    """One contact row as it lives in the database."""

    def __init__(self, **fields) -> None:
        self.fields = fields
        self.archived = False
        self.cancelled = False
        self.lines = []
