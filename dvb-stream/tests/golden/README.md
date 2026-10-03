Goldens for W1-R-low-a. Generated from `main` at commit 182d03f78bd5 with:

    GOLDEN_UPDATE=1 cargo test --locked -p dvb-stream --all-features --test golden_events

Compared byte-for-byte by the same test without `GOLDEN_UPDATE`. Only Task 4
(`transport_header.golden`, parameter order) may regenerate one, after a reviewed diff.
