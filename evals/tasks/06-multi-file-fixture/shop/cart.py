from shop.settings import TAX


def total(prices):
    return round(sum(prices) * (1 + TAX), 2)
