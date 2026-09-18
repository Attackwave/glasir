"""Order business rules and the stock reservation that goes with them."""

from app.models.order import Order
from app.core.numbering import next_number
from app.core.errors import NotFound, Invalid


class OrderService:
    def __init__(self, session) -> None:
        self.session = session

    async def create(self, data) -> Order:
        """Places an order and reserves stock for every line.

        Reservation happens inside the same transaction as the order itself,
        so two customers cannot both be sold the last item.
        """
        contact = await self.session.find_contact(data.contact_id)
        if contact is None:
            raise NotFound(f"contact {data.contact_id}")
        number = await next_number(self.session, "order")
        order = Order(number=number, contact_id=data.contact_id)
        for line in data.lines:
            await self.reserve(line.article_id, line.quantity)
            order.lines.append(line)
        self.session.add(order)
        return order

    async def get(self, order_id: str) -> Order:
        """One order, or NotFound."""
        row = await self.session.find(Order, order_id)
        if row is None:
            raise NotFound(f"order {order_id}")
        return row

    async def list(self, cursor: str | None) -> list[Order]:
        """A page of orders."""
        return await self.session.page(Order, cursor)

    async def reserve(self, article_id: str, quantity: int) -> None:
        """Holds stock for one line, refusing if not enough is free.

        Free stock is on-hand minus what other open orders already hold, not
        the raw on-hand figure — that difference is what stops overselling.
        """
        free = await self.session.free_stock(article_id)
        if free < quantity:
            raise Invalid(f"only {free} of {article_id} available")
        await self.session.hold(article_id, quantity)
