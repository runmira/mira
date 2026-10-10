import unittest

from shop import settings
from shop.cart import total
from shop.receipt import receipt


class Shop(unittest.TestCase):
    def test_renamed(self):
        self.assertTrue(hasattr(settings, "VAT_RATE"))
        self.assertFalse(hasattr(settings, "TAX"))

    def test_rate(self):
        self.assertEqual(settings.VAT_RATE, 0.2)
        self.assertEqual(total([10, 5]), 18.0)
        self.assertEqual(receipt([10]), "12.00 incl. 20% tax")


if __name__ == "__main__":
    unittest.main()
