; P-B: import two separate files that each define #Module Helper.
; Hypothesis (docs alpha.21+): each imported file has its own module-name set,
; so GroupA's Helper and GroupB's Helper are distinct.
; RESULT (alpha.30): "P-B A=A B=B"  -> groups ISOLATE same-named sub-modules.

#Import GroupA
#Import GroupB
FileAppend("P-B A=" GroupA.GetVal() " B=" GroupB.GetVal() "`n", "*")
