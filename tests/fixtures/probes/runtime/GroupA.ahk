; Group A (shared by P-B and P-C): primary module re-exports its OWN same-named
; sub-module "Helper". NB block bodies, not fat-arrow -- `export X() => ...`
; parses as a *call* to export (documented back-compat trap), so nothing exports.
#Import Helper
export GetVal() {
    return Helper.Val()
}

#Module Helper
export Val() {
    return "A"
}
