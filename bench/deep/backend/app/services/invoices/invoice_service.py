"""Invoice business rules, including the parts tax law fixes for us."""

from app.models.invoice import Invoice
from app.core.numbering import next_number
from app.core.errors import NotFound, Invalid


class InvoiceService:
    def __init__(self, session) -> None:
        self.session = session

    async def create(self, data) -> Invoice:
        """Issues an invoice for an order and freezes its totals.

        The totals are computed once and stored, never recomputed on read: an
        invoice is a legal document, so it must keep saying what it said when
        it was issued even after a price list changes.
        """
        order = await self.session.find_order(data.order_id)
        if order is None:
            raise NotFound(f"order {data.order_id}")
        number = await next_number(self.session, "invoice")
        net, tax = self.totals(order)
        invoice = Invoice(number=number, net=net, tax=tax)
        self.session.add(invoice)
        return invoice

    async def get(self, invoice_id: str) -> Invoice:
        """One invoice, or NotFound."""
        row = await self.session.find(Invoice, invoice_id)
        if row is None:
            raise NotFound(f"invoice {invoice_id}")
        return row

    async def list(self, cursor: str | None) -> list[Invoice]:
        """A page of invoices, newest first."""
        return await self.session.page(Invoice, cursor)

    def totals(self, order) -> tuple[int, int]:
        """Net and tax in cents, rounded per line rather than on the sum.

        Rounding the sum instead is off by a cent often enough that customers
        notice, and the tax office reconciles per line.
        """
        net = 0
        tax = 0
        for line in order.lines:
            line_net = line.quantity * line.unit_price
            net += line_net
            tax += round(line_net * line.tax_rate / 100)
        return net, tax

    async def cancel(self, invoice_id: str) -> Invoice:
        """Cancels by issuing a reversal, never by deleting the original."""
        original = await self.get(invoice_id)
        if original.cancelled:
            raise Invalid("already cancelled")
        original.cancelled = True
        return original
