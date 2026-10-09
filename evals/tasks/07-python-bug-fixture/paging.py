def page(items, number, size):
    """Items on page `number` (1-based) when split into pages of `size`."""
    start = number * size
    return items[start:start + size]


def page_count(total, size):
    return total // size
