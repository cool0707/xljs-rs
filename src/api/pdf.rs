//! The natives of the global `pdf` (editing PDF files; see src/pdf).

use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, JsLifetime, Value};

use super::js::{self, elems};
use super::{A, Host, N, R};
use crate::pdf::{PdfDoc, TextOptions};
use crate::{Error, err};

/// The handle of an open PDF document in JavaScript.
#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename = "PdfDocument")]
pub struct Handle {
    #[qjs(skip_trace)]
    pub id: u64,
}

fn handle_id(v: &Value) -> Option<u64> {
    let c = Class::<Handle>::from_object(v.as_object()?)?;
    Some(c.borrow().id)
}

fn track<'js>(h: &Host, ctx: &Ctx<'js>, d: PdfDoc) -> R<'js> {
    let id = h.next_id();
    h.pdfs.borrow_mut().insert(id, d);
    Ok(Class::instance(ctx.clone(), Handle { id })?.into_value())
}

fn id_of(a: &A, i: usize) -> rquickjs::Result<u64> {
    handle_id(&a.arg(i)).ok_or_else(|| a.throw(&err!("pdfdoc.invalidHandle")))
}

/// Runs f with the open document of the handle (argument 0).
fn with_doc<'js, T>(h: &Host, a: &A<'js>, f: impl FnOnce(&mut PdfDoc) -> rquickjs::Result<T>) -> rquickjs::Result<T> {
    let id = id_of(a, 0)?;
    let mut docs = h.pdfs.borrow_mut();
    let Some(d) = docs.get_mut(&id) else { return Err(a.throw(&err!("pdfdoc.invalidHandle"))) };
    if d.closed {
        return Err(a.throw(&err!("pdfdoc.closed")));
    }
    f(d)
}

/// Page indexes (0-based) of argument i, checked against the page count.
fn indexes(a: &A, i: usize, count: usize) -> rquickjs::Result<Vec<usize>> {
    elems(&a.arg(i))
        .iter()
        .map(|v| {
            let n = js::to_f64(v);
            if n >= 0.0 && n < count as f64 && n.fract() == 0.0 {
                Ok(n as usize)
            } else {
                Err(a.throw(&err!("pdfdoc.badPage", ((n + 1.0) as i64).to_string(), count.to_string())))
            }
        })
        .collect()
}

fn closed() -> PdfDoc {
    let mut d = PdfDoc::create();
    d.closed = true;
    d
}

pub(super) fn install(n: &N) {
    n.set("pdfOpen", |h, a| {
        let path = a.str(0);
        let data = std::fs::read(&path).map_err(|e| a.throw(&Error::path("open", path.clone(), e)))?;
        let d = PdfDoc::open(&data, &a.str(1), path.clone()).map_err(|e| super::throw_path(&a.ctx, &path, e))?;
        track(h, &a.ctx, d)
    });
    n.set("pdfOpenBytes", |h, a| {
        let d = PdfDoc::open(&a.bytes(0)?, &a.str(1), String::new()).map_err(|e| a.throw(&e))?;
        track(h, &a.ctx, d)
    });
    // The size (mm, portrait) of a paper name ("A4", "B5", "Letter"), or null.
    n.set("pdfPaperSize", |_, a| match super::xlsx_print::paper_code(&a.str(0)) {
        Some(c) => {
            let (w, h) = super::xlsx_print::paper_size(c);
            a.ret(vec![w.min(h), w.max(h)])
        }
        None => a.null(),
    });
    n.set("pdfCreate", |h, a| track(h, &a.ctx, PdfDoc::create()));
    n.set("pdfPath", |h, a| {
        let p = with_doc(h, a, |d| Ok(d.path.clone()))?;
        if p.is_empty() { a.null() } else { a.ret(p) }
    });
    n.set("pdfPageCount", |h, a| {
        let n = with_doc(h, a, |d| Ok(d.pages.len()))?;
        a.ret(n as f64)
    });
    n.set("pdfPages", |h, a| {
        let pages = with_doc(h, a, |d| {
            Ok((0..d.pages.len())
                .map(|i| {
                    let p = d.page_info(i);
                    serde_json::json!({ "width": p.width, "height": p.height, "rotation": p.rotation })
                })
                .collect::<Vec<_>>())
        })?;
        a.ret(serde_json::Value::Array(pages).to_string())
    });
    // pdfInsert(h, at, src, [indexes of src]): copies the pages of src before index at.
    n.set("pdfInsert", |h, a| {
        let (dst, src) = (id_of(a, 0)?, id_of(a, 2)?);
        let at = a.int(1).max(0) as usize;
        let mut docs = h.pdfs.borrow_mut();
        if docs.get(&dst).is_none_or(|d| d.closed) || docs.get(&src).is_none_or(|d| d.closed) {
            return Err(a.throw(&err!("pdfdoc.closed")));
        }
        if dst == src {
            let d = docs.get_mut(&dst).unwrap();
            let idx = indexes(a, 3, d.pages.len())?;
            d.finish().map_err(|e| a.throw(&e))?;
            let copy = d.doc.clone();
            let ids: Vec<_> = idx.iter().map(|&i| d.pages[i]).collect();
            d.insert_pages(at, &copy, &ids);
        } else {
            let [Some(d), Some(s)] = docs.get_disjoint_mut([&dst, &src]) else { unreachable!() };
            let idx = indexes(a, 3, s.pages.len())?;
            // Text added to src goes along with its pages.
            s.finish().map_err(|e| a.throw(&e))?;
            let ids: Vec<_> = idx.iter().map(|&i| s.pages[i]).collect();
            d.insert_pages(at, &s.doc, &ids);
        }
        a.undefined()
    });
    n.set("pdfInsertBlank", |h, a| {
        let (at, w, ht) = (a.int(1).max(0) as usize, a.f64(2), a.f64(3));
        with_doc(h, a, |d| {
            d.insert_blank(at, w, ht);
            Ok(())
        })?;
        a.undefined()
    });
    n.set("pdfRemove", |h, a| {
        with_doc(h, a, |d| {
            let idx = indexes(a, 1, d.pages.len())?;
            d.remove_pages(&idx);
            Ok(())
        })?;
        a.undefined()
    });
    n.set("pdfRotate", |h, a| {
        let deg = a.int(2);
        with_doc(h, a, |d| {
            let idx = indexes(a, 1, d.pages.len())?;
            d.rotate_pages(&idx, deg);
            Ok(())
        })?;
        a.undefined()
    });
    // pdfAddText(h, page index, text, options JSON)
    n.set("pdfAddText", |h, a| {
        let text = a.str(2);
        let o: TextOptions = serde_json::from_str(&a.str(3)).map_err(|e| a.throw(&Error::msg(e.to_string())))?;
        with_doc(h, a, |d| {
            let idx = indexes(a, 1, d.pages.len())?;
            for i in idx {
                d.add_text(i, &text, &o).map_err(|e| a.throw(&e))?;
            }
            Ok(())
        })?;
        a.undefined()
    });
    n.set("pdfSave", |h, a| {
        let mut path = a.str(1);
        with_doc(h, a, |d| {
            if path.is_empty() {
                path = d.path.clone();
            }
            if path.is_empty() {
                return Err(a.throw(&err!("pdfdoc.noDestination")));
            }
            let data = d.to_bytes().map_err(|e| a.throw(&e))?;
            std::fs::write(&path, data).map_err(|e| a.throw(&Error::path("open", path.clone(), e)))?;
            d.path = path.clone();
            Ok(())
        })?;
        a.ret(path)
    });
    n.set("pdfToBytes", |h, a| {
        let data = with_doc(h, a, |d| d.to_bytes().map_err(|e| a.throw(&e)))?;
        js::array_buffer(&a.ctx, data)
    });
    n.set("pdfClose", |h, a| {
        if let Some(id) = handle_id(&a.arg(0)) {
            // A closed handle keeps reporting "closed".
            h.pdfs.borrow_mut().insert(id, closed());
        }
        a.undefined()
    });
}
