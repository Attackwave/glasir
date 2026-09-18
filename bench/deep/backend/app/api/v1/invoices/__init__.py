"""HTTP routes for invoices."""

from app.schemas.invoice import InvoiceCreate, InvoiceResponse
from app.services.invoices.invoice_service import InvoiceService
from app.core.session import get_session


async def create_invoice(body: InvoiceCreate) -> InvoiceResponse:
    """Issues an invoice against an existing order."""
    session = get_session()
    return InvoiceResponse.of(await InvoiceService(session).create(body))


async def get_invoice(invoice_id: str) -> InvoiceResponse:
    """One invoice by id."""
    session = get_session()
    return InvoiceResponse.of(await InvoiceService(session).get(invoice_id))


async def list_invoices(cursor: str | None = None) -> list[InvoiceResponse]:
    """A page of invoices."""
    session = get_session()
    return [InvoiceResponse.of(r) for r in await InvoiceService(session).list(cursor)]
