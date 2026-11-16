# Rust throughout; the GUI is a Rune window isolated in one crate

Carried over from the 2026-11-13 plan. Rust, because capture and the parsers already are. Rune (Anomaly's native GPU UI framework) for the GUI, because the owner chose it and every Rune app can run headless, so agents can build and check the GUI themselves.

Rune is pre-release, takes no pull requests, and is pinned to one `dev` rev. Only the GUI crate depends on it and the GUI holds no logic, which keeps the bet cheap to reverse. A session view with blurred findings asks more of Rune than the earlier dashboard did, so it is prototyped before the GUI is built. The prototype (#84) found that the pinned rev cannot blur part of a line, so a blur is drawn as a cover chip that shows the finding's kind until clicked; the value never enters the element tree.
