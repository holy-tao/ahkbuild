; P-C: same as P-B, but GroupA/GroupB are loaded from embedded RCDATA resources
; instead of files -- what the .exe backend would emit. Quoted resource spec +
; `as` alias so the name binds into __Main (quoted import does not bind by itself).
; RESULT (alpha.30): "P-C A=A B=B"  -> *RESNAME imports behave exactly like files.

#Import "*GROUPA" as GroupA
#Import "*GROUPB" as GroupB
FileAppend("P-C A=" GroupA.GetVal() " B=" GroupB.GetVal() "`n", "*")
