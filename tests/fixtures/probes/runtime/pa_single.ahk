; P-A: two #Module Helper blocks in ONE file.
; Hypothesis: the second reopens/merges the first (shared namespace).
; GetX() is defined in the first block; X is defined in the second block.
; If merged -> GetX() can see X -> prints 123. If isolated -> error/blank.
; RESULT (alpha.30): "P-A GetX()=123"  -> same-file blocks MERGE.

#Import Helper {GetX}
FileAppend("P-A GetX()=" GetX() "`n", "*")

#Module Helper
GetX() => X

#Module Helper
global X := 123
