# Spelling dictionaries

`en_US.aff` and `en_US.dic` are the American English Hunspell dictionary made from SCOWL
(version 2020.12.07, SCOWL size 60), as packaged in the `dictionary-en` 4.0.0 npm package.
`en_GB.dic` (British, -ise spellings), `en_CA.dic` and `en_AU.dic` come from the same SCOWL
release, as packaged in `dictionary-en-gb`, `dictionary-en-ca` and `dictionary-en-au` 3.0.0.
Their affix files are identical to `en_US.aff`, so only the word lists are kept.

The licences, permissive (MIT and BSD-style), are in the `LICENSE-*.txt` files. Panna reads
the files with its own small affix expander (`src/spell.rs`); nothing else is used from the
packages.
