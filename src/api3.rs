use failure::*;
use std::collections::HashMap;


use crate::json_schema::*;
use crate::api_info::*;
use serde_json::{json, Value};


static GET: ApiMethod = ApiMethod {
    handler: test_api_handler,
    description: "This is a simple test.",
    parameters: &Object!{
        properties => &propertymap!{
            force => &Boolean!{
                optional => Some(true),
                description => "Test for boolean options."
            }
        }
    },
    returns: &Jss::Null,
};

fn test_api_handler(param: Value) -> Result<Value, Error> {
    println!("This is a test {}", param);

   // let force: Option<bool> = Some(false);

    //if let Some(force) = param.force {
    //}

    let _force =  param["force"].as_bool()
        .ok_or_else(|| format_err!("missing parameter 'force'"))?;

    if let Some(_force) = param["force"].as_bool() {
    }


    Ok(json!(null))
}

pub fn get_api_definition() -> MethodInfo {

    let subdir1 = methodinfo!{
        get => &GET
    };
    
    let info = methodinfo!{
        get => &GET,
        subdirs => {
            let mut map = HashMap::new();

            map.insert(String::from("subdir"), subdir1);
            
            map
        }
    };

    info
}
