#!/bin/sh
# Download the test fonts too big to keep in git: Noto for the scripts font
# fallback is tested on, and Unicode's Last Resort font. They land in
# fonts/fetched/ (git-ignored), each checked against its SHA-256, so every
# machine tests against the same bytes. Files already there and correct are
# left alone, so running this again costs nothing.
#
# The tests that need these fonts run this themselves when one is missing,
# so `cargo test` is enough on a fresh checkout (with a network).
#
# CALAMUS_FONT_MIRROR=https://... fetches every file from <mirror>/<name>
# instead, for when the fonts are hosted somewhere of tolege's own.
#
# All are SIL Open Font License 1.1.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
dir="$here/fetched"
mkdir -p "$dir"

noto=https://raw.githubusercontent.com/notofonts/notofonts.github.io/4cf78353cd1668040ba8efa13029cd910fcdf9e2/fonts
cjk=https://raw.githubusercontent.com/notofonts/noto-cjk/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/Japanese
resort=https://github.com/unicode-org/last-resort-font/releases/download/18.000

# name, SHA-256, where from
fonts="
NotoSans-Regular.ttf 478c558ea716033cd60c03438f628dfa75694dcf6b5f6d505a2f05fd2b4f3823 $noto/NotoSans/hinted/ttf
NotoSansArabic-Regular.ttf bdff3e5659d67e67def05b33f749683b9376ae819d65d3dd62ac4640b3aaef48 $noto/NotoSansArabic/hinted/ttf
NotoSansDevanagari-Regular.ttf 4e3c66638958c3e2ab5d37f47a8deb89fffeb7be9985c665a519bbc7ba762313 $noto/NotoSansDevanagari/hinted/ttf
NotoSansHebrew-Regular.ttf cdefaf8efd47045f6820928eba84db5bed7557539328952b5f828315485e02ee $noto/NotoSansHebrew/hinted/ttf
NotoSansThai-Regular.ttf 61cf814eec46b294d6ea4401ac295d0cecd5207bd2331dcc5a15e7301d30ee44 $noto/NotoSansThai/hinted/ttf
NotoSansCJKjp-Regular.otf 68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5 $cjk
LastResortHE-Regular.ttf cb7c2262cb86a33e7bdfabd0a51746e738b0dd10924126c4530c414c5fc7e4b6 $resort
"

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

echo "$fonts" | while read -r name want from; do
    [ -n "$name" ] || continue
    file="$dir/$name"
    if [ -f "$file" ] && [ "$(sha "$file")" = "$want" ]; then
        continue
    fi
    url="${CALAMUS_FONT_MIRROR:-$from}/$name"
    echo "fetching $name" >&2
    curl -fsSL --retry 3 -o "$file.part" "$url"
    got=$(sha "$file.part")
    if [ "$got" != "$want" ]; then
        rm -f "$file.part"
        echo "$name from $url has SHA-256 $got, not $want" >&2
        exit 1
    fi
    mv "$file.part" "$file"
done
