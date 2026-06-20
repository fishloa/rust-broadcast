# MMI — High-Level objects (text / enq / answ / menu / menu_answ / list)

_Source: EN 50221 §8.6.5, Tables 46-51 + value tables (PDF pp. 47-50), render-verified_

High-Level MMI mode gives the application higher-level semantics (menus and lists);
the host determines the look and feel of the display. Text in high-level mode is coded
per reference [4] (ETSI EN 300 468 Annex A character sets).

## Table 46 — Text object coding (text)

apdu_tag pair: Ttext-last `9F 88 03` / Ttext-more `9F 88 04`, Direction `<---`.
Used as a component inside higher-level objects.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `text() {` | | |
| &nbsp;&nbsp;text_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;text_char | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

A text object with `text_length` = 0 is a null object: nothing is displayed.
`text_char` is coded per reference [4]; may include control characters.

## Table 47 — Enq object coding (enq)

apdu_tag `Tenq` = `9F 88 07`, Direction `<---`. Requests a single user input
(e.g. a PIN). The host returns the response in the Answ object.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `enq () {` | | |
| &nbsp;&nbsp;enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;blind_answer | 1 | bslbf |
| &nbsp;&nbsp;answer_text_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<enq_length-2; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;text_char | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- `blind_answer` — set to 1 means the user input is not to be displayed when entered
  (the host chooses the replacement character, e.g. star).
- `answer_text_length` — expected length of the answer. Set to hex `FF` if unknown by
  the application.
- `text_char` — coded per reference [4].

## Table 48 — Answ object coding (answ)

apdu_tag `Tansw` = `9F 88 08`, Direction `--->`. Used with the Enq object to return
the user input.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `answ () {` | | |
| &nbsp;&nbsp;answ_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;answ_id | 8 | uimsbf |
| &nbsp;&nbsp;`if (answ_id == answer) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;text_char | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

### answ_id values (Table, p. 48)

| answ_id | value |
|---------|-------|
| cancel  | `00` |
| answer  | `01` |
| reserved | other values |

`answer` — the object contains the user input (may be zero length). `cancel` — the
user wishes to abort the dialogue. The text_chars in Answ use the same character
coding scheme / signalling as the associated Enq object.

## Table 49 — Menu object coding (menu)

apdu_tag pair: Tmenu_last `9F 88 09` / Tmenu_more `9F 88 0A`, Direction `<---`.
Used with the Menu Answ object to manage menus in high-level MMI mode.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `menu () {` | | |
| &nbsp;&nbsp;menu_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;choice_nb | 8 | uimsbf |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* title text */ | | |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* sub-title text */ | | |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* bottom text */ | | |
| &nbsp;&nbsp;`for (i=0; i<choice_nb; i++) {`&nbsp;&nbsp;/* when choice_nb != 'FF' */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;TEXT() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

A menu is one Title, one sub-title, several choices and one bottom line. TEXT objects
with text_length = 0 may be used (e.g. no sub-title / no bottom text). `choice_nb` =
`FF` means this field does not carry the number of choices; the host then reads
choices until the end of the object.

## Table 50 — Menu Answ object coding (menu_answ)

apdu_tag `Tmenu_answ` = `9F 88 0B`, Direction `--->`. Returns the user choice;
also used with the list object to indicate the user has finished with it.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `menu_answ () {` | | |
| &nbsp;&nbsp;menu_answ_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;choice_ref | 8 | uimsbf |
| `}` | | |

- `choice_ref` — the number of the choice selected by the user. `choice_ref` = `01`
  is the first choice presented (first choice text after the bottom text in the menu
  object), `02` the second, etc.
- `choice_ref` = `00` indicates the user cancelled the preceding menu or list object
  without making a choice.

## Table 51 — List object coding (list)

apdu_tag pair: Tlist_last `9F 88 0C` / Tlist_more `9F 88 0D`, Direction `<---`.
Sends a list of items to be displayed (e.g. entitlements). Same syntax as the Menu
object; used with the Menu Answ object.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `list () {` | | |
| &nbsp;&nbsp;list_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;item_nb | 8 | uimsbf |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* title text */ | | |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* sub-title text */ | | |
| &nbsp;&nbsp;TEXT()&nbsp;&nbsp;/* bottom text */ | | |
| &nbsp;&nbsp;`for (i=0; i<item_nb; i++) {`&nbsp;&nbsp;/* when item_nb != 'FF' */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;TEXT() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

A list is one Title, one sub-title, several items and one bottom line. TEXT objects
with text_length = 0 may be used. `item_nb` = `FF` means this field does not carry the
number of items.
