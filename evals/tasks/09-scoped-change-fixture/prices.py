def apply_discount(price, percent):
    return price - price * percent / 100


def format_price(price):
    return "$" + str(price)
