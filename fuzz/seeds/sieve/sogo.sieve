require ["vacation","fileinto"];
# Filter: Invoices
if header :contains "subject" "invoice" { fileinto "Archive"; stop; }
/* dated */
if allof(currentdate :value "ge" "date" "2026-12-01") {
    vacation :days 7 :addresses ["carol@mc31.test"] :subject "Away \"till\" Monday" text:
Back on Monday.
..signed
.
;
}
