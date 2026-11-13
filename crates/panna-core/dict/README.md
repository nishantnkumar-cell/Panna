# Spelling dictionary

`en_US.aff` and `en_US.dic` are the American English Hunspell dictionary made from SCOWL
(version 2020.12.07, SCOWL size 60), as packaged in the `dictionary-en` 4.0.0 npm package.
Their licence, permissive (MIT and BSD-style), is in `LICENSE-en_US.txt`. Panna reads them
with its own small affix expander (`src/spell.rs`); nothing else is used from the package.
