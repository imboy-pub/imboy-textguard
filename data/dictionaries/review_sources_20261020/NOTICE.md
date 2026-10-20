# Third-party data and modifications

Source repositories and fixed revisions are recorded in provenance/sources.json and tools/lexicon/sources.lock.json.

- kirklin/go-swd: Apache-2.0, Copyright [2024-PRESENT] [Kirk Lin <https://github.com/kirklin>]. Original licence is preserved in licenses/kirklin-Apache-2.0.txt.
- konsheng/Sensitive-lexicon: MIT, Copyright (c) 2024~2099 Konsheng. Original licence is preserved in licenses/konsheng-MIT.txt.
- houbb/sensitive-word-data: repository LICENSE.txt declares Apache-2.0 and includes its original attribution/translation; the complete file is preserved in licenses/houbb-Apache-2.0.txt.

Modifications on 2026-10-20: conservative signal-based filtering, exclusion of inappropriate/personal/contact material and malformed literal patterns, base-normalized deduplication, narrow categorization, stable generated rule identifiers, TextGuard JSON conversion and profile-only review configuration. No source words were synthesized to reach a quota. Repository declarations are not an independent audit of all historical third-party provenance. No warranty of data correctness, legality of matched text, business accuracy or zero false positives is made. The project licence does not replace these source licences.
