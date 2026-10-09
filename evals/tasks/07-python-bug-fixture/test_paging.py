import unittest

from paging import page, page_count


class Paging(unittest.TestCase):
    def test_first_page(self):
        self.assertEqual(page(list(range(10)), 1, 3), [0, 1, 2])

    def test_last_page(self):
        self.assertEqual(page(list(range(10)), 4, 3), [9])

    def test_count(self):
        self.assertEqual(page_count(10, 3), 4)
        self.assertEqual(page_count(9, 3), 3)
        self.assertEqual(page_count(0, 3), 0)


if __name__ == "__main__":
    unittest.main()
