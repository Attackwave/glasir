"""Contact business rules.

Everything that decides something about a contact lives here rather than in
the route above: name validation, the running contact number, and the archive
rule that keeps a contact readable after it stops being used.
"""

from app.models.contact import Contact
from app.core.numbering import next_number
from app.core.errors import NotFound, Invalid


class ContactService:
    def __init__(self, session) -> None:
        self.session = session

    async def create(self, data) -> Contact:
        """Validates the name fields, assigns the next contact number and stores it.

        A contact is either a company or a person, never both and never
        neither — that is the one rule this layer refuses to bend, because
        every report downstream groups by it.
        """
        self._validate_name(data)
        number = await next_number(self.session, "contact")
        contact = Contact(number=number, company_name=data.company_name)
        self.session.add(contact)
        return contact

    async def get(self, contact_id: str) -> Contact:
        """One contact, or NotFound. Archived contacts are still returned."""
        row = await self.session.find(Contact, contact_id)
        if row is None:
            raise NotFound(f"contact {contact_id}")
        return row

    async def list(self, cursor: str | None) -> list[Contact]:
        """A page of contacts, excluding archived ones unless asked."""
        return await self.session.page(Contact, cursor, exclude_archived=True)

    async def archive(self, contact_id: str) -> Contact:
        """Marks a contact archived instead of deleting it.

        Deleting would orphan every invoice and order that references it, so
        an archived contact stays readable and stops appearing in lists.
        """
        contact = await self.get(contact_id)
        contact.archived = True
        return contact

    def _validate_name(self, data) -> None:
        has_company = bool(data.company_name)
        has_person = bool(data.last_name)
        if has_company == has_person:
            raise Invalid("a contact is either a company or a person")
