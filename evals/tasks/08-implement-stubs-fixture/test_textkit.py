import unittest

from textkit.count import word_count
from textkit.slug import slugify
from textkit.title import title_case
from textkit.wrap import wrap


class TextKit(unittest.TestCase):
    def test_slug(self):
        self.assertEqual(slugify("Hello, World!  2026"), "hello-world-2026")

    def test_wrap(self):
        self.assertEqual(wrap("the quick brown fox jumps", 10), "the quick\nbrown fox\njumps")

    def test_count(self):
        self.assertEqual(word_count("  one two\tthree\n"), 3)
        self.assertEqual(word_count(""), 0)

    def test_title(self):
        self.assertEqual(title_case("the lord of the rings"), "The Lord of the Rings")


if __name__ == "__main__":
    unittest.main()
