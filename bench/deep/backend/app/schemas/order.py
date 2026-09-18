"""Request and response shapes for orders."""


class OrderCreate:
    """What a caller sends to place an order."""

    contact_id: str
    lines: list


class OrderLineCreate:
    """One line on an order being placed."""

    article_id: str
    quantity: int
    unit_price: int
    tax_rate: int


class OrderResponse:
    """What a route returns for one order."""

    @staticmethod
    def of(row):
        """Builds a response from a stored row."""
        return OrderResponse()
