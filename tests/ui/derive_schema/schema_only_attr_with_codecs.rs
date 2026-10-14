use manivelle::TsSchema;

#[derive(TsSchema)]
struct Invoice {
    #[manivelle(rename = "invoiceId")]
    invoice_id: String,
}

fn main() {}
