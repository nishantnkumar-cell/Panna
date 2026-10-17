# App icon

`make_icon.py` builds the icon SVG. Its colours match the Chitra icon (indigo-to-plum tile, coral-to-peach accents, dark hexagon gem). The P, D and F are glyph outlines from **Inter ExtraBold** (SIL Open Font License 1.1), converted to paths so the icon needs no font at runtime.

```sh
pip install fonttools
python3 packaging/icon/make_icon.py /path/to/Inter-ExtraBold.otf packaging/linux/io.github.nishantnkumar_cell.Panna.svg
```
