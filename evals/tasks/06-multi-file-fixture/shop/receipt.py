from shop import settings
from shop.cart import total


def receipt(prices):
    return f"{total(prices):.2f} incl. {int(settings.TAX * 100)}% tax"
