"""Request and response shapes for invoices."""


class InvoiceCreate:
    """What a caller sends to issue an invoice."""

    order_id: str


class InvoiceLineCreate:
    """One line on an invoice being issued."""

    article_id: str
    quantity: int


class InvoiceResponse:
    """What a route returns for one invoice."""

    @staticmethod
    def of(row):
        """Builds a response from a stored row."""
        return InvoiceResponse()
