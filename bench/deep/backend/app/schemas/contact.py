"""Request and response shapes for contacts.

These are transport types, not the model: they exist so a route can reject a
malformed body before any business rule sees it.
"""


class ContactCreate:
    """What a caller must send to register a contact."""

    company_name: str | None
    last_name: str | None


class ContactPersonCreate:
    """A person attached to a company contact."""

    first_name: str
    last_name: str


class AddressCreate:
    """A postal address on a contact."""

    street: str
    city: str


class BankAccountCreate:
    """Bank details on a contact, validated for IBAN shape only."""

    iban: str


class ContactResponse:
    """What a route returns for one contact."""

    @staticmethod
    def of(row):
        """Builds a response from a stored row."""
        return ContactResponse()
