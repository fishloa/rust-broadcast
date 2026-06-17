# Table 4: Coding of packet X/28/0 Format 1 for basic Level 1 pages

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-4 | Page Function = Basic Level 1 Teletext page (see clause 9.4.2.1).  |
|  1 | 5-7 | Page Coding = All 8-bit bytes, each comprising 7 bits data and 1 odd parity bit (see clause 9.4.2.1).  |
|  1 | 8-14 | Default G0 and G2 Character Set Designation and National Option Selection Default G0 primary and G2 supplementary character sets plus national option character sub-sets are designated. The 7-bit value is used to select an entry in table 32. NOTE: The default character sets at the start of each row are the default G0 and G2 sets. In some transmissions, each "ESC" control character (code 1/B) on the Level 1 page toggles the G0 set between the default and second G0 sets for the subsequent G0 characters of the row.  |
|  1 | 15-18 | Second G0 Set Designation and National Option Selection  |
|  2 | 1-3 | A second G0 character set and a national option sub-set are designated. The 7-bit value is used to select an entry in table 33. See previous note.  |
|  2 | 4 | Left Side Panel 0 = No left side panel is to be displayed; 1 = Left side panel is to be displayed.  |
|  2 | 5 | Right Side Panel 0 = No right side panel is to be displayed; 1 = Right side panel is to be displayed.  |
|  2 | 6 | Side Panel Status Flag 0 = Side panel(s) required at Level 3.5 only; 1 = Side panel(s) required at Levels 2.5 & 3.5.  |
|  2 | 7-10 | Number of Columns in Side Panels Bits 7 to 10 (LSB to MSB) define the number of columns in the left side panel. If the right side-panel is to be displayed, its width (in columns) is 16 minus this value. When only one side panel is in use, a value of 0 indicates a side panel of 16 columns.  |
|  2 | 11-18 | Colour Map Entry Coding for CLUTs 2 and 3  |
|  3-12 | 1-18 | The bits are organized as 16 data words, each of 12 bits. Each word defines an entry in the Colour Map of clause 12.4, proceeding in transmission order from CLUT 2, entry 0 to CLUT 3, entry 7. Each 12-bit data word contains 4 bits for each primary colour (Red, Green and Blue), in the transmission order: RRRRGGGGBBBB, with ascending order of bit significance within each 4 bits.  |
|  13 | 1-4 |   |
|  13 | 5-9 | Default Screen Colour Selects an entry in the Colour Map of clause 12.4 to be applied to the screen area above display row 0 and below row 23, or 24 if used. Screen colour selection via a packet X/26 takes priority over this value.  |
|  13 | 10-14 | Default Row Colour Selects an entry in the Colour Map of clause 12.4 to be applied to rows 0 to 23, and 24 where used. Row colour selection via a packet X/26 takes priority over this value.  |
|  13 | 15 | Black Background Colour Substitution This bit controls the substitution of black background colour on the Level 1 page by the pertaining full row colour. 0 = No substitution of black background by the pertaining row colour. NOTE: This black background may still be substituted by another colour as a result of the Colour Table Re-mapping function, see below. 1 = On any row where the Level 1 page displays a black background as a result of the start-of-row default or the spacing attribute Black Background (1/C), the black background is replaced by the full row colour applying to that row. This substitution takes place independently of any colour table re-mapping that may be applied by the function described below. This substitution does not occur as a result of the spacing attribute sequence Alpha (or Mosaics) Black (0/0 or 1/0) followed by New Background (1/D). Where background colour is used as a parameter in the determination of the operation of another function, for example colour table re-mapping, colour table flash and Level 2.5 and 3.5 windows, it shall be set explicitly by the transmission and not depend upon the result of a black background colour substitution invoked by this bit.  |
|  13 | 16-18 | Colour Table Re-mapping for use with Spacing Attributes Allows colour table re-mapping of the spacing colour attributes used on the Level 1 page. Foreground and background colours may be mapped independently to different CLUTs within the Colour Map of clause 12.4 according to the following table. The entry in the selected CLUT is specified by the 3 LSBs of the code for the spacing colour attribute.  |



|  Triplet | Bits | Function  |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- |
|   |  | Colour Table Re-mapping |   |   |   |   |
|   |  | Bit |   | Foreground | Background |   |
|   |  | 18 | 17 | 16 | CLUT | CLUT  |
|   |  | 0 | 0 | 0 | 0 | 0  |
|   |  | 0 | 0 | 1 | 0 | 1  |
|   |  | 0 | 1 | 0 | 0 | 2  |
|   |  | 0 | 1 | 1 | 1 | 1  |
|   |  | 1 | 0 | 0 | 1 | 2  |
|   |  | 1 | 0 | 1 | 2 | 1  |
|   |  | 1 | 1 | 0 | 2 | 2  |
|   |  | 1 | 1 | 1 | 2 | 3  |
|   |  | NOTE: If Black Background Colour Substitution is in force, a background colour of black (entry number 0) on the Level 1 page is only re-mapped by this technique if the black background was set as a result of the spacing attribute sequence Alpha (or Mosaics) Black (0/0 or 1/0) followed by New Background (1/D).  |   |   |   |   |

# 9.4.2.3 Coding for data broadcasting pages

The coding of table 5 applies to the data bits of a packet X/28/0 Format 1 when the Page Function bits indicate a page a data broadcasting page (code 0001).
