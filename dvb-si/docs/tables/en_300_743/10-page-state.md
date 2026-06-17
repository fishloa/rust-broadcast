# Table 10: Page state

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Page state | Effect on page | Comments  |
| --- | --- | --- | --- |
|  00 | normal case | page update | The display set contains only the subtitle elements that are changed from the previous page instance.  |
|  01 | acquisition point | page refresh | The display set contains all subtitle elements needed to display the next page instance.  |
|  10 | mode change | new page | The display set contains all subtitle elements needed to display the new page.  |
|  11 | reserved |  | Reserved for future use.  |

If the page state is "mode change" or "acquisition point", then the display set shall contain a region composition segment for each region used in this epoch.

processed_length: The total number of bytes that have already been processed following the segment_length field.

region_id: This uniquely identifies a region within a page. Each identified region is displayed in the page instance defined in this page composition. Regions shall be listed in the page_composition_segment in the order of ascending region_vertical_address field values.

region_horizontal_address: This specifies the horizontal address of the top left pixel of this region. The left-most pixel of the active pixels has horizontal address zero, and the pixel address increases from left to right.

region_vertical_address: This specifies the vertical address of the top line of this region. The top line of the frame is line zero, and the line address increases by one within the frame from top to bottom.

NOTE: All addressing of pixels is based on a frame of M pixels horizontally by N scan lines vertically. These numbers are independent of the aspect ratio of the picture; on a 16:9 display a pixel looks a bit wider than on a 4:3 display. In some cases, for instance a logo, this may lead to unacceptable distortion. Separate data may be provided for presentation on each of the different aspect ratios. The subtitling descriptor signals whether the associated subtitle data can be presented on any display or on displays of specific aspect ratio only.



## 7.2.3 Region composition segment

The region composition for a specific region is carried in region_composition_segments. The region composition contains a list of objects; the listed objects shall be positioned in such a way that they do not overlap.

If an object is added to a region in case of a page update, new pixel data will overwrite either the background colour of the region or "old objects". The programme provider shall take care that the new pixel data overwrites only information that needs to be replaced, but also that it overwrites all pixels in the region that are not to be preserved. Note that a pixel is either defined by the background colour, or by an "old" object or by a "new" object; if a pixel is overwritten none of its previous definition is retained.

Table 11 shows the syntax of the region composition segment.
