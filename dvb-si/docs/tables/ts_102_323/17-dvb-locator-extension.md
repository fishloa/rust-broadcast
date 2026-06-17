## Table 17 — DVB locator extension
_§6.4, PDF pp. 30-31_

| dvb_event_constraint | = event_id_mode \| tva_id_only_mode \| time_constraint |
|---|---|
| event_id_mode | = ";" event_id [ ";" TVA_id ] [ time_constraint ] |
| tva_id_only_mode | = [ "." component_tag ] ";;" TVA_id [ time_constraint ] |
| time_constraint | = "~" time_duration |
| TVA_id | = 1\*hex |
| time_duration | = start_time "--" duration |
| dvb-entity | = dvb_transport_stream \| dvb_service \| dvb_service_component \| dvb_carousel |
| dvb_carousel | = dvb_service_without_event "$" carousel_id |
| carousel_id | = 1\*hex |
| start_time | = date "T" time "Z" |
| duration | = "PT" hours "H" minutes "M" [ seconds "S"] |
| date | = year month day |
| time | = hours minutes [ seconds ] |
| year | = digit digit digit digit |
| month | = digit digit |
| day | = digit digit |
| hours | = digit digit |
| minutes | = digit digit |
| seconds | = digit digit |
| hex | = digit \| "A" \| "B" \| "C" \| "D" \| "E" \| "F" \| "a" \| "b" \| "c" \| "d" \| "e" \| "f" |
| digit | = "0" \| "1" \| "2" \| "3" \| "4" \| "5" \| "6" \| "7" \| "8" \| "9" |

