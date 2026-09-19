//! A real optional plugin: one state, command/tool entry points, portable/rich documents.
wit_bindgen::generate!({path:"../policy.wit",world:"policy"});
use exports::misa::policy::policy_api::*;
use serde_json::{Value, json};
struct Pet;
fn fault(message: &str) -> Fault {
    Fault {
        code: "pet.invalid".into(),
        message: message.into(),
        event: None,
    }
}
fn request(id: &str) -> QueryRequest {
    QueryRequest {
        id: id.into(),
        args: vec![],
    }
}
impl Guest for Pet {
    fn describe() -> Descriptor {
        Descriptor{
            id:"pet".into(),version:"0.3.0".into(),events:vec!["plugin.pet.feed".into(),"plugin.pet.ask-feed".into()],effects:vec![],roots:vec!["pet".into()],
            commands:vec![CommandDefinition{request:None,id:"pet.feed".into(),event:"plugin.pet.feed".into(),input:r#"{"type":"record","fields":{"amount":{"schema":{"type":"int"}}}}"#.into()},
                CommandDefinition{id:"pet.ask-feed".into(),event:"plugin.pet.ask-feed".into(),input:r#"{"type":"record","fields":{}}"#.into(),request:Some(r#"{"title":"Feed the pet","input":{"type":"record","fields":{"amount":{"schema":{"type":"int"}}}},"fields":{"amount":{"label":"Treats (1–10)"}}}"#.into())}],
            tools:vec![ToolBinding{name:"pet_feed".into(),description:"Feed the shared session pet with 1 to 10 treats".into(),command:"pet.feed".into()},ToolBinding{name:"pet_ask_feed".into(),description:"Ask the owner how many treats to feed".into(),command:"pet.ask-feed".into()}],
            bindings:vec![ActionBinding{id:"pet.feed".into(),command:"pet.feed".into(),bound:r#"{"amount":1}"#.into(),inputs:"{}".into()},ActionBinding{id:"pet.ask-feed".into(),command:"pet.ask-feed".into(),bound:"{}".into(),inputs:"{}".into()}],
            queries:vec![
                QueryDefinition{id:"pet.state".into(),contract:"pet.state@1".into(),arguments:vec![],output:r#"{"kind":"data","schema":{"type":"record","fields":{"treats":{"schema":{"type":"int"}}}}}"#.into(),source:QuerySource::Read(ReadContract{roots:vec!["pet".into()],schema:r#"{"type":"record","fields":{"pet":{"schema":{"type":"record","fields":{},"allow_unknown":true}}}}"#.into()})},
                QueryDefinition{id:"pet.portable".into(),contract:"pet.portable@1".into(),arguments:vec![],output:r#"{"kind":"document"}"#.into(),source:QuerySource::Derived(vec![request("pet.state")])},
                QueryDefinition{id:"pet.rich".into(),contract:"pet.rich@1".into(),arguments:vec![],output:r#"{"kind":"document"}"#.into(),source:QuerySource::Derived(vec![request("pet.state")])},
            ],
            presentations:vec![Presentation{id:"companion".into(),title:"Session pet".into(),variants:vec![
                PresentationVariant{id:"rich".into(),requirements:vec!["semantic.meter@1".into()],query:request("pet.rich")},
                PresentationVariant{id:"portable".into(),requirements:vec![],query:request("pet.portable")},
            ]}],
        }
    }
    fn configure(_options: Vec<OptionValue>) -> Result<(), Fault> {
        Ok(())
    }
    fn handle(event: Event, db: String) -> Result<(Vec<Patch>, Vec<Effect>), Fault> {
        if event.kind != "plugin.pet.feed" && event.kind != "plugin.pet.ask-feed" {
            return Err(fault("Unknown pet event"));
        }
        let data: Value = serde_json::from_str(event.data.as_deref().unwrap_or("null"))
            .map_err(|_| fault("Invalid event data"))?;
        let input = if event.kind == "plugin.pet.ask-feed" {
            &data["input"]["value"]
        } else {
            &data["input"]
        };
        let amount = input["amount"]
            .as_i64()
            .filter(|amount| (1..=10).contains(amount))
            .ok_or_else(|| fault("Feed between 1 and 10 treats"))?;
        let db: Value = serde_json::from_str(&db).map_err(|_| fault("Invalid state"))?;
        let treats = db["pet"]["treats"]
            .as_i64()
            .unwrap_or(0)
            .checked_add(amount)
            .ok_or_else(|| fault("Treat count overflow"))?;
        Ok((
            vec![Patch {
                path: "pet.treats".into(),
                op: Op::Set(treats.to_string()),
            }],
            vec![],
        ))
    }
    fn query(
        request: QueryRequest,
        inputs: Vec<String>,
        read_data: Option<String>,
        _previous: Option<String>,
    ) -> Result<String, Fault> {
        if request.id == "pet.state" {
            let data: Value =
                serde_json::from_str(&read_data.ok_or_else(|| fault("Missing declared data"))?)
                    .map_err(|_| fault("Invalid data"))?;
            return Ok(json!({"treats":data["pet"]["treats"].as_i64().unwrap_or(0)}).to_string());
        }
        if read_data.is_some() {
            return Err(fault("Derived document received undeclared data"));
        }
        let state: Value =
            serde_json::from_str(inputs.first().ok_or_else(|| fault("Missing state input"))?)
                .map_err(|_| fault("Invalid state input"))?;
        let treats = state["treats"].as_i64().unwrap_or(0);
        let kind = match request.id.as_str() {
            "pet.portable" => {
                json!({"shape":"status","text":format!("Pet · {treats} treats · {}",if treats==0{"hungry"}else{"happy"})})
            }
            "pet.rich" => {
                json!({"shape":"meter","label":format!("Pet happiness · {treats} treats"),"value":treats.min(10),"max":10})
            }
            _ => return Err(fault("Unknown pet query")),
        };
        Ok(json!({"id":"pet","role":"pet.companion","kind":kind,"actions":[{"id":"pet.feed","label":"Feed"},{"id":"pet.ask-feed","label":"Choose treats"}]}).to_string())
    }
}
export!(Pet);
