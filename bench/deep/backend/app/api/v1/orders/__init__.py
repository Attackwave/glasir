"""HTTP routes for orders."""

from app.schemas.order import OrderCreate, OrderResponse
from app.services.orders.order_service import OrderService
from app.core.session import get_session


async def create_order(body: OrderCreate) -> OrderResponse:
    """Places an order for a contact and reserves its stock."""
    session = get_session()
    return OrderResponse.of(await OrderService(session).create(body))


async def get_order(order_id: str) -> OrderResponse:
    """One order by id."""
    session = get_session()
    return OrderResponse.of(await OrderService(session).get(order_id))


async def list_orders(cursor: str | None = None) -> list[OrderResponse]:
    """A page of orders."""
    session = get_session()
    return [OrderResponse.of(r) for r in await OrderService(session).list(cursor)]
