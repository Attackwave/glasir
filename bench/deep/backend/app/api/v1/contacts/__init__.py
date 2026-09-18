"""HTTP routes for contacts.

The handler layer is deliberately thin: it validates the request body, calls
the service and returns the schema. Anything that decides something belongs in
the service below it.
"""

from app.schemas.contact import ContactCreate, ContactResponse
from app.services.contacts.contact_service import ContactService
from app.core.session import get_session


async def create_contact(body: ContactCreate) -> ContactResponse:
    """Registers a new contact and returns it with its assigned number."""
    session = get_session()
    contact = await ContactService(session).create(body)
    return ContactResponse.of(contact)


async def get_contact(contact_id: str) -> ContactResponse:
    """Looks up one contact by id, or raises if it does not exist."""
    session = get_session()
    return ContactResponse.of(await ContactService(session).get(contact_id))


async def list_contacts(cursor: str | None = None) -> list[ContactResponse]:
    """A page of contacts, newest first, continued by an opaque cursor."""
    session = get_session()
    rows = await ContactService(session).list(cursor)
    return [ContactResponse.of(r) for r in rows]
