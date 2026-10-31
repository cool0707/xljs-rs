use rquickjs::Value;

use super::N;
use super::js::{self, object, v};
use crate::vba;

pub(super) fn install(n: &N) {
    // vbaRead(path) / vbaReadBytes(bytes, name): the VBA project as
    // { name, codePage, modules: [{ name, type, code, sheet }], text }, or
    // null without macros. text is vba::text, named after the file.
    fn result<'js>(c: &rquickjs::Ctx<'js>, p: Option<vba::Project>, file: &str) -> super::R<'js> {
        let Some(p) = p else { return Ok(Value::new_null(c.clone())) };
        let mods: Vec<Value> = p
            .modules
            .iter()
            .map(|m| {
                let sheet = if m.sheet.is_empty() { Value::new_null(c.clone()) } else { v(c, m.sheet.clone()) };
                object(c, vec![("name", v(c, m.name.clone())), ("type", v(c, m.kind.clone())), ("code", v(c, m.code.clone())), ("sheet", sheet)]).unwrap()
            })
            .collect();
        let text = vba::text(file, &p);
        object(c, vec![("name", v(c, p.name)), ("codePage", v(c, p.code_page)), ("modules", js::array(c, mods)?), ("text", v(c, text))])
    }
    n.set("vbaRead", |_, a| {
        let path = a.str(0);
        let p = vba::read_file(&path).map_err(|e| a.throw(&e))?;
        result(&a.ctx, p, &super::gopath::base(&path))
    });
    n.set("vbaReadBytes", |_, a| {
        let p = vba::read(&a.bytes(0)?).map_err(|e| a.throw(&e))?;
        result(&a.ctx, p, &a.str(1))
    });
}
