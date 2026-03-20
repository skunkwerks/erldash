use crate::error;
use erl_dist::node::NodeName;
use erl_dist::term::{Atom, FixInteger, List, Map, Term, Tuple};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct SystemVersion(String);

impl nojson::DisplayJson for SystemVersion {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for SystemVersion {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        Ok(SystemVersion(value.try_into()?))
    }
}

impl SystemVersion {
    pub fn get(&self) -> &str {
        &self.0
    }
}

pub fn find_cookie() -> error::Result<String> {
    if let Some(dir) = dirs::home_dir().filter(|dir| dir.join(".erlang.cookie").exists()) {
        let cookie = std::fs::read_to_string(dir.join(".erlang.cookie"))?;
        Ok(cookie)
    } else {
        Err(error::Error::new(
            "Could not find the cookie file $HOME/.erlang.cookie. Please specify `-cookie` arg instead.",
        ))
    }
}

#[derive(Debug, Clone)]
pub struct RpcClient {
    handle: erl_rpc::RpcClientHandle,
}

impl RpcClient {
    pub async fn connect(
        erlang_node: &NodeName,
        port: Option<u16>,
        cookie: &str,
    ) -> error::Result<Self> {
        let client = if let Some(port) = port {
            erl_rpc::RpcClient::connect_with_port(&erlang_node.to_string(), port, cookie).await?
        } else {
            erl_rpc::RpcClient::connect(&erlang_node.to_string(), cookie).await?
        };
        let handle = client.handle();
        smol::spawn(async {
            if let Err(e) = client.run().await {
                log::error!("Erlang RPC Client error: {e}");
            }
        })
        .detach();

        Ok(Self { handle })
    }

    pub async fn get_system_version(&self) -> error::Result<SystemVersion> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_info".into(),
                List::from(vec![Atom::from("system_version").into()]),
            )
            .await?;
        term_to_string(term).map(SystemVersion)
    }

    pub async fn get_system_info_u64(&self, item_name: &str) -> error::Result<u64> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_info".into(),
                List::from(vec![Atom::from(item_name).into()]),
            )
            .await?;
        term_to_u64(term)
    }

    pub async fn get_statistics_1st_u64(&self, item_name: &str) -> error::Result<u64> {
        let term = self.get_statistics(item_name).await?;
        term_to_tuple_1st_u64(term)
    }

    pub async fn get_statistics_u64_list(&self, item_name: &str) -> error::Result<Vec<u64>> {
        let term = self.get_statistics(item_name).await?;
        term_to_u64_list(term)
    }

    pub async fn get_statistics_io(&self) -> error::Result<(u64, u64)> {
        let term = self.get_statistics("io").await?;
        let tuple = term_to_tuple(term)?;
        let in_bytes = term_to_tuple_2nd_u64(tuple.elements[0].clone())?;
        let out_bytes = term_to_tuple_2nd_u64(tuple.elements[1].clone())?;
        Ok((in_bytes, out_bytes))
    }

    pub async fn get_statistics_microstate_accounting(&self) -> error::Result<Vec<MSAccThread>> {
        let term = self.get_statistics("microstate_accounting").await?;
        term_to_list(term)?
            .elements
            .into_iter()
            .map(MSAccThread::from_term)
            .collect()
    }

    pub async fn set_system_flag_bool(&self, name: &str, value: &str) -> error::Result<bool> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "system_flag".into(),
                List::from(vec![Atom::from(name).into(), Atom::from(value).into()]),
            )
            .await?;
        term_to_bool(term)
    }

    pub async fn get_memory(&self) -> error::Result<BTreeMap<String, u64>> {
        let term = self
            .handle
            .clone()
            .call("erlang".into(), "memory".into(), List::nil())
            .await?;
        term_to_list(term)?
            .elements
            .into_iter()
            .map(|x| {
                let tuple = term_to_tuple(x)?;
                if tuple.elements.len() != 2 {
                    return Err(error::Error::new(format!(
                        "expected a two-elements tuple, but got {}",
                        tuple
                    )));
                }
                let key = term_to_atom(tuple.elements[0].clone())?;
                let value = term_to_u64(tuple.elements[1].clone())?;
                Ok((key.name, value))
            })
            .collect()
    }

    pub async fn get_recon_info(&self, id: u32, serial: u32) -> error::Result<Term> {
        let pid_tuple = Tuple::from(vec![
            FixInteger::from(0i32).into(),
            FixInteger::from(id as i32).into(),
            FixInteger::from(serial as i32).into(),
        ]);
        let term = self
            .handle
            .clone()
            .call(
                "recon".into(),
                "info".into(),
                List::from(vec![pid_tuple.into()]),
            )
            .await?;
        Ok(term)
    }

    pub async fn get_recon_proc_count(
        &self,
        attribute: &str,
        count: i32,
    ) -> error::Result<Vec<ReconProcInfo>> {
        let term = self
            .handle
            .clone()
            .call(
                "recon".into(),
                "proc_count".into(),
                List::from(vec![
                    Atom::from(attribute).into(),
                    FixInteger::from(count).into(),
                ]),
            )
            .await?;
        term_to_list(term)?
            .elements
            .into_iter()
            .map(ReconProcInfo::from_term)
            .collect()
    }

    async fn get_statistics(&self, item_name: &str) -> error::Result<Term> {
        let term = self
            .handle
            .clone()
            .call(
                "erlang".into(),
                "statistics".into(),
                List::from(vec![Atom::from(item_name).into()]),
            )
            .await?;
        Ok(term)
    }
}

fn term_to_tuple_1st_u64(term: Term) -> error::Result<u64> {
    let tuple = term_to_tuple(term)?;
    if tuple.elements.is_empty() {
        return Err(error::Error::new(format!(
            "expected a non empty tuple, but got {}",
            tuple
        )));
    }
    term_to_u64(tuple.elements[0].clone())
}

fn term_to_tuple_2nd_u64(term: Term) -> error::Result<u64> {
    let tuple = term_to_tuple(term)?;
    if tuple.elements.len() < 2 {
        return Err(error::Error::new(format!(
            "expected a tuple having 2 or more elements, but got {}",
            tuple
        )));
    }
    term_to_u64(tuple.elements[1].clone())
}

fn term_to_pid_string(term: Term) -> error::Result<String> {
    if let Term::Pid(pid) = term {
        Ok(format!("<0.{}.{}>", pid.id, pid.serial))
    } else {
        Err(error::Error::new(format!(
            "expected a pid, but got {}",
            term
        )))
    }
}

fn term_to_u64(term: Term) -> error::Result<u64> {
    let v = match term {
        Term::FixInteger(v) => v.value.try_into()?,
        Term::BigInteger(v) => v.value.try_into()?,
        v => return Err(error::Error::new(format!("{} is not an integer", v))),
    };
    Ok(v)
}

fn term_to_string(term: Term) -> error::Result<String> {
    match term {
        Term::ByteList(bl) => Ok(String::from_utf8(bl.bytes)?),
        other => {
            let bytes = term_to_list(other)?
                .elements
                .into_iter()
                .map(term_to_u8)
                .collect::<error::Result<Vec<_>>>()?;
            Ok(String::from_utf8(bytes)?)
        }
    }
}

fn term_to_u8(term: Term) -> error::Result<u8> {
    if let Term::FixInteger(v) = term {
        Ok(u8::try_from(v.value)?)
    } else {
        Err(error::Error::new(format!(
            "expected an integer, but got {}",
            term
        )))
    }
}

fn term_to_u64_list(term: Term) -> error::Result<Vec<u64>> {
    match term {
        Term::ByteList(bl) => Ok(bl.bytes.into_iter().map(u64::from).collect()),
        other => term_to_list(other)?
            .elements
            .into_iter()
            .map(term_to_u64)
            .collect(),
    }
}

fn term_to_bool(term: Term) -> error::Result<bool> {
    let atom = term_to_atom(term)?;
    match atom.name.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(error::Error::new(format!(
            "expected 'true' or 'false', but got {}",
            atom.name
        ))),
    }
}

fn term_to_atom(term: Term) -> error::Result<Atom> {
    term.try_into()
        .map_err(|x: Term| error::Error::new(format!("expected an atom, but got {x}")))
}

fn term_to_tuple(term: Term) -> error::Result<Tuple> {
    term.try_into()
        .map_err(|x: Term| error::Error::new(format!("expected a tuple, but got {x}")))
}

fn term_to_list(term: Term) -> error::Result<List> {
    term.try_into()
        .map_err(|x: Term| error::Error::new(format!("expected a list, but got {x}")))
}

#[derive(Debug, Clone)]
pub struct MSAccThread {
    pub thread_id: u64,
    pub thread_type: String,
    pub counters: BTreeMap<String, u64>,
}

impl MSAccThread {
    fn from_term(term: Term) -> error::Result<Self> {
        let map: Map = term
            .try_into()
            .map_err(|x: Term| error::Error::new(format!("expected a map, but got {x}")))?;
        let mut thread_id = None;
        let mut thread_type = None;
        let mut counters = BTreeMap::new();
        for (k, v) in map.map {
            match term_to_atom(k)?.name.as_str() {
                "id" => {
                    thread_id = Some(term_to_u64(v)?);
                }
                "type" => {
                    thread_type = Some(term_to_atom(v)?.name);
                }
                "counters" => {
                    let counters_map: Map = v.try_into().map_err(|x: Term| {
                        error::Error::new(format!("expected a map, but got {x}"))
                    })?;
                    for (k, v) in counters_map.map {
                        counters.insert(term_to_atom(k)?.name, term_to_u64(v)?);
                    }
                }
                k => {
                    log::debug!("unknown msacc key: {:?}", k);
                }
            }
        }
        Ok(Self {
            thread_id: thread_id.ok_or_else(|| error::Error::new("missing 'id' key"))?,
            thread_type: thread_type.ok_or_else(|| error::Error::new("missing 'type' key"))?,
            counters,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReconProcInfo {
    pub pid: String,
    pub value: u64,
    pub registered_name: Option<String>,
    pub current_function: String,
}

impl ReconProcInfo {
    fn from_term(term: Term) -> error::Result<Self> {
        let tuple = term_to_tuple(term)?;
        if tuple.elements.len() != 3 {
            return Err(error::Error::new(format!(
                "expected a 3-element tuple, but got {}",
                tuple
            )));
        }
        let pid = term_to_pid_string(tuple.elements[0].clone())?;
        let value = term_to_u64(tuple.elements[1].clone())?;
        let info_list = term_to_list(tuple.elements[2].clone())?;

        let mut registered_name = None;
        let mut current_function = String::new();

        for elem in info_list.elements {
            if let Ok(atom) = term_to_atom(elem.clone()) {
                registered_name = Some(atom.name);
                continue;
            }
            if let Ok(kv_tuple) = term_to_tuple(elem)
                && kv_tuple.elements.len() == 2
                    && let Ok(key) = term_to_atom(kv_tuple.elements[0].clone())
                        && key.name == "current_function"
                            && let Ok(mfa) = term_to_tuple(kv_tuple.elements[1].clone())
                                && mfa.elements.len() == 3
                                    && let (Ok(module), Ok(function), Ok(arity)) = (
                                        term_to_atom(mfa.elements[0].clone()),
                                        term_to_atom(mfa.elements[1].clone()),
                                        term_to_u64(mfa.elements[2].clone()),
                                    ) {
                                        current_function =
                                            format!("{}:{}/{}", module.name, function.name, arity);
                                    }
        }

        Ok(Self {
            pid,
            value,
            registered_name,
            current_function,
        })
    }
}

pub fn format_recon_info(term: Term) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(categories) = term_to_list(term) else {
        lines.push("(unexpected format)".to_string());
        return lines;
    };
    for cat_term in categories.elements {
        let Ok(cat_tuple) = term_to_tuple(cat_term) else {
            continue;
        };
        if cat_tuple.elements.len() != 2 {
            continue;
        }
        let category = term_to_atom(cat_tuple.elements[0].clone())
            .map(|a| a.name)
            .unwrap_or_else(|_| format_term_value(&cat_tuple.elements[0]));
        lines.push(format!("== {} ==", category));

        if let Ok(items) = term_to_list(cat_tuple.elements[1].clone()) {
            for item_term in items.elements {
                if let Ok(kv) = term_to_tuple(item_term)
                    && kv.elements.len() == 2 {
                        let key = term_to_atom(kv.elements[0].clone())
                            .map(|a| a.name)
                            .unwrap_or_else(|_| format_term_value(&kv.elements[0]));
                        let value = &kv.elements[1];

                        if key == "current_stacktrace" {
                            lines.push(format!("  {}:", key));
                            format_stacktrace(value, &mut lines);
                        } else {
                            lines.push(format!("  {}: {}", key, format_term_value(value)));
                        }
                    }
            }
        }
        lines.push(String::new());
    }
    lines
}

fn format_stacktrace(term: &Term, lines: &mut Vec<String>) {
    if let Ok(frames) = term_to_list(term.clone()) {
        for frame in &frames.elements {
            if let Ok(t) = term_to_tuple(frame.clone()) {
                if t.elements.len() >= 3 {
                    let module = term_to_atom(t.elements[0].clone())
                        .map(|a| a.name)
                        .unwrap_or_else(|_| format_term_value(&t.elements[0]));
                    let function = term_to_atom(t.elements[1].clone())
                        .map(|a| a.name)
                        .unwrap_or_else(|_| format_term_value(&t.elements[1]));
                    let arity = format_term_value(&t.elements[2]);

                    let location = if t.elements.len() >= 4 {
                        format_file_line(&t.elements[3])
                    } else {
                        String::new()
                    };
                    if location.is_empty() {
                        lines.push(format!("    {}:{}/{}", module, function, arity));
                    } else {
                        lines.push(format!(
                            "    {}:{}/{} ({})",
                            module, function, arity, location
                        ));
                    }
                } else {
                    lines.push(format!("    {}", format_term_value(frame)));
                }
            } else {
                lines.push(format!("    {}", format_term_value(frame)));
            }
        }
    } else {
        lines.push(format!("    {}", format_term_value(term)));
    }
}

fn format_file_line(term: &Term) -> String {
    if let Ok(props) = term_to_list(term.clone()) {
        let mut file = None;
        let mut line = None;
        for prop in &props.elements {
            if let Ok(t) = term_to_tuple(prop.clone())
                && t.elements.len() == 2
                    && let Ok(key) = term_to_atom(t.elements[0].clone()) {
                        match key.name.as_str() {
                            "file" => file = Some(format_term_value(&t.elements[1])),
                            "line" => line = Some(format_term_value(&t.elements[1])),
                            _ => {}
                        }
                    }
        }
        match (file, line) {
            (Some(f), Some(l)) => format!("{}:{}", f, l),
            (Some(f), None) => f,
            _ => String::new(),
        }
    } else {
        String::new()
    }
}

pub(crate) fn format_term_value(term: &Term) -> String {
    match term {
        Term::Atom(a) => a.name.clone(),
        Term::FixInteger(i) => i.value.to_string(),
        Term::BigInteger(i) => i.value.to_string(),
        Term::Float(f) => format!("{}", f.value),
        Term::Pid(p) => format!("<0.{}.{}>", p.id, p.serial),
        Term::Tuple(t) => {
            if t.elements.len() == 3
                && let (Ok(m), Ok(f), Ok(_)) = (
                    term_to_atom(t.elements[0].clone()),
                    term_to_atom(t.elements[1].clone()),
                    term_to_u64(t.elements[2].clone()),
                ) {
                    return format!(
                        "{}:{}/{}",
                        m.name,
                        f.name,
                        format_term_value(&t.elements[2])
                    );
                }
            let inner: Vec<String> = t.elements.iter().map(format_term_value).collect();
            format!("{{{}}}", inner.join(", "))
        }
        Term::List(l) => {
            if l.elements.is_empty() {
                return "[]".to_string();
            }
            let inner: Vec<String> = l.elements.iter().map(format_term_value).collect();
            let joined = inner.join(", ");
            if joined.len() > 120 {
                let mut s = String::from("[\n");
                for item in &inner {
                    s.push_str(&format!("      {}\n", item));
                }
                s.push_str("    ]");
                s
            } else {
                format!("[{}]", joined)
            }
        }
        Term::Map(m) => {
            let inner: Vec<String> = m
                .map
                .iter()
                .map(|(k, v)| format!("{} => {}", format_term_value(k), format_term_value(v)))
                .collect();
            format!("#{{{}}}", inner.join(", "))
        }
        Term::Binary(b) => match String::from_utf8(b.bytes.clone()) {
            Ok(s) => format!("<<{:?}>>", s),
            Err(_) => format!("<<{} bytes>>", b.bytes.len()),
        },
        Term::ByteList(bl) => match String::from_utf8(bl.bytes.clone()) {
            Ok(s) => format!("{:?}", s),
            Err(_) => format!("[{} bytes]", bl.bytes.len()),
        },
        _ => format!("{}", term),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erl_dist::term::{Pid as ErlPid, Term};

    fn make_pid(id: u32, serial: u32) -> Term {
        ErlPid::new("nonode@nohost".to_string(), id, serial, 0).into()
    }

    fn make_mfa(module: &str, function: &str, arity: i32) -> Term {
        Tuple::from(vec![
            Atom::from(module).into(),
            Atom::from(function).into(),
            FixInteger::from(arity).into(),
        ])
        .into()
    }

    fn make_recon_proc(
        id: u32,
        serial: u32,
        value: i32,
        reg_name: Option<&str>,
        mfa: (&str, &str, i32),
    ) -> Term {
        let mut info_elems: Vec<Term> = Vec::new();
        if let Some(name) = reg_name {
            info_elems.push(Atom::from(name).into());
        }
        let current_fn_kv = Tuple::from(vec![
            Atom::from("current_function").into(),
            make_mfa(mfa.0, mfa.1, mfa.2),
        ]);
        info_elems.push(current_fn_kv.into());
        Tuple::from(vec![
            make_pid(id, serial),
            FixInteger::from(value).into(),
            List::from(info_elems).into(),
        ])
        .into()
    }

    // --- term_to_pid_string ---

    #[test]
    fn pid_string_format() {
        assert_eq!(
            term_to_pid_string(make_pid(42, 7)).unwrap(),
            "<0.42.7>"
        );
    }

    #[test]
    fn pid_string_rejects_non_pid() {
        assert!(term_to_pid_string(Atom::from("not_a_pid").into()).is_err());
    }

    // --- ReconProcInfo::from_term ---

    #[test]
    fn recon_proc_info_named_process() {
        let term = make_recon_proc(42, 1, 1024, Some("my_proc"), ("erlang", "self", 0));
        let info = ReconProcInfo::from_term(term).unwrap();
        assert_eq!(info.pid, "<0.42.1>");
        assert_eq!(info.value, 1024);
        assert_eq!(info.registered_name, Some("my_proc".to_string()));
        assert_eq!(info.current_function, "erlang:self/0");
    }

    #[test]
    fn recon_proc_info_unnamed_process() {
        let term = make_recon_proc(100, 5, 512, None, ("gen_server", "loop", 7));
        let info = ReconProcInfo::from_term(term).unwrap();
        assert_eq!(info.pid, "<0.100.5>");
        assert_eq!(info.value, 512);
        assert_eq!(info.registered_name, None);
        assert_eq!(info.current_function, "gen_server:loop/7");
    }

    #[test]
    fn recon_proc_info_rejects_wrong_arity_tuple() {
        let bad = Tuple::from(vec![make_pid(1, 0), FixInteger::from(0i32).into()]);
        assert!(ReconProcInfo::from_term(bad.into()).is_err());
    }

    // --- format_term_value ---

    #[test]
    fn format_term_value_atom() {
        assert_eq!(format_term_value(&Atom::from("hello").into()), "hello");
    }

    #[test]
    fn format_term_value_integer() {
        assert_eq!(
            format_term_value(&FixInteger::from(42i32).into()),
            "42"
        );
    }

    #[test]
    fn format_term_value_pid() {
        assert_eq!(format_term_value(&make_pid(42, 7)), "<0.42.7>");
    }

    #[test]
    fn format_term_value_mfa_tuple() {
        let mfa = make_mfa("erlang", "self", 0);
        assert_eq!(format_term_value(&mfa), "erlang:self/0");
    }

    #[test]
    fn format_term_value_generic_tuple() {
        let t = Tuple::from(vec![
            Atom::from("a").into(),
            Atom::from("b").into(),
        ]);
        assert_eq!(format_term_value(&t.into()), "{a, b}");
    }

    #[test]
    fn format_term_value_empty_list() {
        assert_eq!(format_term_value(&List::nil().into()), "[]");
    }

    // --- format_recon_info ---

    #[test]
    fn format_recon_info_single_category() {
        let mem_kv = Tuple::from(vec![
            Atom::from("memory").into(),
            FixInteger::from(1024i32).into(),
        ]);
        let bin_kv = Tuple::from(vec![
            Atom::from("binary").into(),
            FixInteger::from(512i32).into(),
        ]);
        let items = List::from(vec![mem_kv.into(), bin_kv.into()]);
        let cat = Tuple::from(vec![Atom::from("memory_info").into(), items.into()]);
        let top = List::from(vec![cat.into()]);

        let lines = format_recon_info(top.into());
        assert!(lines.contains(&"== memory_info ==".to_string()));
        assert!(lines.contains(&"  memory: 1024".to_string()));
        assert!(lines.contains(&"  binary: 512".to_string()));
    }

    #[test]
    fn format_recon_info_stacktrace() {
        let frame = Tuple::from(vec![
            Atom::from("erlang").into(),
            Atom::from("apply").into(),
            FixInteger::from(2i32).into(),
            List::nil().into(),
        ]);
        let stacktrace_kv = Tuple::from(vec![
            Atom::from("current_stacktrace").into(),
            List::from(vec![frame.into()]).into(),
        ]);
        let items = List::from(vec![stacktrace_kv.into()]);
        let cat = Tuple::from(vec![Atom::from("meta").into(), items.into()]);
        let top = List::from(vec![cat.into()]);

        let lines = format_recon_info(top.into());
        assert!(lines.iter().any(|l| l.contains("current_stacktrace")));
        assert!(lines.iter().any(|l| l.contains("erlang:apply/2")));
    }

    #[test]
    fn format_recon_info_invalid_input() {
        let lines = format_recon_info(Atom::from("not_a_list").into());
        assert_eq!(lines, vec!["(unexpected format)"]);
    }
}
