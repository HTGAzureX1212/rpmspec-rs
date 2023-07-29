use crate::{error::ParserError as PE, parse::SpecParser, util::Consumer};
use color_eyre::eyre::eyre;
use parking_lot::Mutex;
use smartstring::alias::String;
use std::{
	collections::HashMap,
	io::{Read, Write},
	path::Path,
	sync::Arc,
};

#[derive(Clone)]
pub enum MacroType {
	Internal(fn(&mut SpecParser, &mut String, &mut Consumer<dyn Read + '_>) -> Result<(), PE>),
	Runtime { file: Arc<Path>, offset: usize, len: usize, s: Arc<Mutex<String>>, param: bool },
}

impl std::fmt::Debug for MacroType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Internal(_) => f.write_str("<builtin>")?,
			Self::Runtime { offset, len, s, .. } => {
				f.write_str(&s.lock()[*offset..*&(offset + len)])?;
			}
		}
		Ok(())
	}
}

impl From<&str> for MacroType {
	fn from(value: &str) -> Self {
		Self::Runtime { file: Arc::from(Path::new("unknown")), offset: 0, s: Arc::new(Mutex::new(value.into())), param: false, len: value.len() }
	}
}

macro_rules! __internal_macros {
	($(macro $m:ident($p:ident, $o:ident, $r:ident) $body:block )+) => {
		$(
			#[allow(non_snake_case)]
			fn $m($p: &mut SpecParser, $o: &mut String, $r: &mut Consumer<dyn Read + '_>) -> Result<(), PE> $body
		)+
		lazy_static::lazy_static! {
			pub static ref INTERNAL_MACROS: HashMap<String, Vec<MacroType>> = {
				let mut ret = HashMap::new();
				$({
					ret.insert(stringify!($m).into(), vec![MacroType::Internal($m)]);
				})+
				ret
			};
		}
	};
}

// you will see some `#[rustfmt::skip]`, this is related to
// https://github.com/rust-lang/rustfmt/issues/5866
__internal_macros!(
	macro define(p, _o, r) {
		while let Some(ch) = r.next() {
			if !ch.is_whitespace() {
				r.back();
				break;
			}
		}
		let pos = r.pos;
		let def = r.read_til_eol().ok_or_else(|| eyre!("%define: read_til_eol() failed"))?;
		let def = def.trim_start();
		#[rustfmt::skip]
		let Some((name, def)) = def.split_once(' ') else {
			return Err(eyre!("%define: Expected 2 arguments").into());
		};
		let def = def.trim();
		let (name, param): (String, bool) = name.strip_suffix("()").map_or_else(|| (name.into(), false), |x| (x.into(), true));
		let csm = r.range(pos + 1 + name.len()..r.pos).ok_or_else(|| eyre!("%define: cannot unwind Consumer"))?;
		p.define_macro(name, csm, param, def.len());
		Ok(())
	}
	macro global(p, o, r) {
		define(p, o, r)
	}
	macro undefine(p, _o, r) {
		p.macros.remove(&r.read_til_eol().unwrap());
		Ok(())
	}
	macro load(p, _o, r) {
		let f: String = r.collect();
		p.load_macro_from_file(&std::path::Path::new(&*f))?;
		Ok(())
	}
	macro expand(p, o, r) {
		// * Why downcasting `r` yet again?
		// Apparently `r` is `dyn` but we need to convert it to `impl` to use `p.parse_macro()`.
		// See `p._rp_macro()` for more info.
		// * Wait wait, won't `parse_macro()` call `_rp_macro()`?
		// Yeah... internal macros should not call `_rp_macro()` but we have no choice...
		// It will skip `Arc::try_unwrap()` inside `_rp_macro()` anyway since `new_reader.r` is
		// `None`. This should be safe.

		let new_reader = r.range(r.pos..r.end).ok_or_else(|| eyre!("Cannot wind Consumer in %expand"))?;

		// SAFETY:
		// This is a valid downcast because `new_reader.r` is `None` given
		// that it is created from `Consumer::range()`. Therefore, changing
		// `<R>` to anything should not affect the actual reader.
		let mut new_reader = *unsafe { Box::from_raw(Box::into_raw(Box::new(new_reader)) as *mut Consumer<std::fs::File>) };
		p.parse_macro(o, &mut new_reader)?;
		// r.pos = new_reader.pos;
		Ok(())
	}
	macro expr(p, o, r) {
		todo!()
	}
	macro lua(p, o, r) {
		let content: String = r.collect();
		let parser = Arc::new(Mutex::new(std::mem::take(p)));
		let out = crate::lua::run(&parser, &content)?;
		std::mem::swap(p, &mut Arc::try_unwrap(parser).expect("Cannot unwrap Arc for print() output in lua").into_inner()); // break down Arc then break down Mutex
		o.push_str(&out);
		Ok(())
	}
	macro macrobody(p, o, r) {
		let name = r.collect();
		#[rustfmt::skip]
		let Some(Some(m)) = p.macros.get(&name).map(|x| x.last()) else {
			return Err(PE::MacroNotFound(name));
		};
		match m {
			MacroType::Internal(_) => o.push_str("<builtin>"),
			MacroType::Runtime { file, offset, len, s, .. } => {
				// we can put anything as <R>
				let mut csm: Consumer<std::fs::File> = Consumer::new(Arc::clone(s), None, Arc::clone(file));
				csm.pos = *offset;
				csm.end = *offset + len;
				o.push_str(&csm.collect::<String>());
			}
		}
		Ok(())
	}
	macro quote(_p, o, r) {
		o.push('"');
		o.push_str(&r.collect::<String>());
		o.push('"');
		Ok(())
	}
	macro gsub(p, o, r) {
		todo!()
	}
	macro len(_p, o, r) {
		o.push_str(&r.collect::<Box<[char]>>().len().to_string());
		Ok(())
	}
	macro lower(_p, o, r) {
		// assume it's ascii?
		o.push_str(&r.collect::<String>().to_ascii_lowercase());
		Ok(())
	}
	macro rep(p, o, r) {
		todo!()
	}
	macro reverse(_p, o, r) {
		let mut chs = r.collect::<Box<[char]>>();
		chs.reverse();
		chs.into_iter().for_each(|ch| o.push(*ch));
		Ok(())
	}
	macro sub(p, o, r) {
		todo!()
	}
	macro upper(_p, o, r) {
		// assume it's ascii?
		o.push_str(&r.collect::<String>().to_ascii_uppercase());
		Ok(())
	}
	macro shescape(_p, o, r) {
		o.push('\'');
		for ch in r {
			if ch == '\'' {
				o.push('\'');
				o.push('\\');
				o.push('\'');
			}
			o.push(ch);
		}
		o.push('\'');
		Ok(())
	}
	macro shrink(_p, o, r) {
		while let Some(ch) = r.next() {
			if !ch.is_whitespace() {
				o.push(ch);
				break;
			}
		}
		let mut space = false;
		for ch in r {
			if ch.is_whitespace() {
				space = true;
				continue;
			}
			if space {
				space = false;
				o.push(' ');
			}
			o.push(ch);
		}
		Ok(())
	}
	macro basename(_p, o, r) {
		// according to testing this has nothing to do with the `basename` command
		let s: String = r.collect();
		o.push_str(s.rsplit_once('/').map_or(&s, |(_, x)| x));
		Ok(())
	}
	macro dirname(_p, o, r) {
		let s: String = r.collect();
		o.push_str(s.rsplit_once('/').map_or(&s, |(x, _)| x));
		Ok(())
	}
	macro exists(_p, o, r) {
		o.push(if Path::new(&*r.collect::<String>()).exists() { '1' } else { '0' });
		Ok(())
	}
	macro suffix(_p, o, r) {
		let s: String = r.collect();
		o.push_str(s.rsplit_once('.').map_or("", |(_, x)| x));
		Ok(())
	}
	macro url2path(_p, o, r) {
		// ? https://github.com/rpm-software-management/rpm/blob/master/rpmio/url.c#L50
		let s: String = r.collect();
		#[rustfmt::skip]
		let Ok(url) = url::Url::parse(&s) else {
			o.push_str(&s);
			return Ok(());
		};
		if matches!(url.scheme(), "https" | "http" | "hkp" | "file" | "ftp") {
			o.push_str(url.path());
		} else {
			o.push_str(&s);
		}
		Ok(())
	}
	macro u2p(p, o, r) {
		url2path(p, o, r)
	}
	macro uncompress(p, o, r) {
		//? https://github.com/rpm-software-management/rpm/blob/master/tools/rpmuncompress.c#L69
		todo!()
	}
	macro getncpus(_p, o, r) {
		if r.next().is_some() {
			r.back();
			tracing::warn!(args=?r.collect::<String>(), "Unnecessary arguments supplied to `%getncpus`.");
		}
		o.push_str(&num_cpus::get().to_string());
		Ok(())
	}
	macro getconfidir(_p, o, _r) {
		let res = std::env::var("RPM_CONFIGDIR");
		if let Err(std::env::VarError::NotUnicode(s)) = res {
			return Err(eyre!("%{{getconfdir}} failed: While grabbing env var `RPM_CONFIGDIR`: Non-unicode OsString {s:?}").into());
		}
		o.push_str(res.as_ref().map(|x| &**x).unwrap_or("/usr/lib/rpm"));
		Ok(())
	}
	macro getenv(_p, o, r) {
		let name: String = r.collect();
		match std::env::var(&*name) {
			Ok(x) => o.push_str(&x),
			Err(std::env::VarError::NotPresent) => {}
			Err(std::env::VarError::NotUnicode(s)) => return Err(eyre!("%{{getenv:{name}}} failed: Non-unicode OsString {s:?}").into()),
		}
		Ok(())
	}
	macro rpmversion(_p, _o, _r) {
		todo!()
	}
	macro echo(_p, _o, r) {
		tracing::info!("{}", r.collect::<String>());
		Ok(())
	}
	macro warn(_p, _o, r) {
		tracing::warn!("{}", r.collect::<String>());
		Ok(())
	}
	macro error(_p, _o, r) {
		tracing::error!("{}", r.collect::<String>());
		Ok(())
	}
	macro verbose(_p, o, _r) {
		// FIXME
		o.push('0');
		Ok(())
	}
	macro S(p, o, r) {
		// FIXME?
		expand(p, o, &mut Consumer::new(Arc::new(Mutex::new("%SOURCE".into())), None, r.file.clone()))?;
		r.for_each(|c| o.push(c));
		Ok(())
	}
	macro P(p, o, r) {
		// FIXME?
		expand(p, o, &mut Consumer::new(Arc::new(Mutex::new("%PATCH".into())), None, r.file.clone()))?;
		r.for_each(|c| o.push(c));
		Ok(())
	}
	macro trace(p, o, r) {
		todo!()
	}
	macro dump(p, _o, r) {
		let args = r.collect::<String>();
		if args.len() != 0 {
			tracing::warn!(?args, "Unexpected arguments to %dump");
		}
		let mut stdout = std::io::stdout().lock();
		for (k, v) in &p.macros {
			if let Some(v) = v.last() {
				if let MacroType::Internal(_) = v {
					stdout.write_fmt(format_args!("[<internal>]\t%{k}\t<builtin>\n"))?;
					continue;
				}
				let MacroType::Runtime { file, offset, len, s, param } = v else { unreachable!() };
				let ss = s.lock();
				let front = &ss[..*offset];
				let nline = front.chars().filter(|c| *c == '\n').count() + 1;
				let col = offset - front.find('\n').unwrap_or(0);
				let f = file.display();
				let p = if *param { "{}" } else { "" };
				let inner = &ss[*offset..*offset+*len];
				stdout.write_fmt(format_args!("[{f}:{nline}:{col}]\t%{k}{p}\t{inner}\n"))?;
			}
		}
		Ok(())
	}
);