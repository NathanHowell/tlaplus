---- MODULE UnicodeLegacy ----
EXTENDS Naturals, TLC

\* Demonstrates Unicode identifiers, strings, and math symbols.
\* Mixed-script comment: café, 東京, Δ, ∀, and 🚀 ensure UTF-8 paths round-trip.

CONSTANT ΩLegacyFlag

VARIABLES 状態, Δcount

TypeInvariant == Δcount ∈ Nat

Init ==
    /\ 状態 = "開始"
    /\ Δcount = 0

StepChoices == {"✓", "✗", "記録"}

Next ==
    ∃ 選択 ∈ StepChoices:
        /\ 状態' = 選択
        /\ Δcount' = Δcount + IF ΩLegacyFlag THEN 1 ELSE 2

UnicodeInvariant ==
    Δcount >= 0

Spec ==
    Init /\ [][Next]_<<状態, Δcount>>

THEOREM Spec => []UnicodeInvariant

====
