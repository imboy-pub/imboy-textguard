"""Offline regression tests for source parsing and classification boundaries."""
import unittest
from build_review_lexicon import blob, classify, constraint, norm, records, parse_ranges


class CollectionTests(unittest.TestCase):
    def test_git_blob_empty(self):
        self.assertEqual(blob(b''), 'e69de29bb2d1d6434b8b29ae775ad8c2e48c5391')

    def test_normalization(self):
        self.assertEqual(norm('ＡB测\u200b试'), 'ab测试')

    def test_tag_parser_preserves_whole_term(self):
        self.assertEqual(list(records('出售海洛因 1,4\n'.encode(), 'tags')), [(1, '出售海洛因', ['1', '4'])])

    def test_invalid_tag_line_fails(self):
        with self.assertRaises(ValueError):
            list(records('没有分类标签'.encode(), 'tags'))

    def test_range_parser(self):
        self.assertEqual(list(parse_ranges('2,5-7')), [2, 5, 6, 7])

    def test_no_synthetic_expansion(self):
        self.assertEqual(list(records('词一,词二\n'.encode(), 'lines')), [(1, '词一,词二', [])])
        self.assertIsNotNone(constraint('词一,词二'))

    def test_reject_contacts_and_pattern_fragments(self):
        for word in ['联系方式13800138000', '裸聊*', '色情{5}', '毛色情', '色情词']:
            self.assertIsNotNone(constraint(word))

    def test_no_identity_or_topic_as_abuse(self):
        for word in ['张三', '政府工作报告', '合法政治讨论', '普通宗教活动', '咖啡因', '警官证']:
            self.assertIsNone(classify(word))

    def test_classification_needs_a_concrete_signal(self):
        self.assertEqual(classify('出售假警官证')[0], 'political_abuse')
        self.assertEqual(classify('出售海洛因')[0], 'drugs')
        self.assertEqual(classify('出售手枪')[0], 'terrorism')
        self.assertEqual(classify('色情网站')[0], 'sexual')
        self.assertIsNotNone(constraint('未成年人'))


if __name__ == '__main__':
    unittest.main()
