//! A line reader for the two cAdvisor disk I/O counter families in Prometheus text. The
//! kubelet body is megabytes of other families, so lines are filtered by prefix before any
//! parsing and the body itself is never kept.

use std::borrow::Cow;
use std::collections::BTreeMap;

const READS_PREFIX: &str = "container_fs_reads_bytes_total{";
const WRITES_PREFIX: &str = "container_fs_writes_bytes_total{";

/// Cumulative disk bytes of one cgroup, summed over devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskIoCounters {
    /// The newest sample time of the series that were summed.
    pub sampled_at: Option<jiff::Timestamp>,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerDiskIo {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub counters: DiskIoCounters,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskIoSample {
    /// The root cgroup (`id="/"`), summed over devices; `None` when cAdvisor has no such series.
    pub node: Option<DiskIoCounters>,
    /// Ordered by (namespace, pod, container).
    pub containers: Vec<ContainerDiskIo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Read,
    Write,
}

struct DiskIoLine<'a> {
    direction: Direction,
    id: Option<Cow<'a, str>>,
    namespace: Option<Cow<'a, str>>,
    pod: Option<Cow<'a, str>>,
    container: Option<Cow<'a, str>>,
    value: u64,
    at: Option<jiff::Timestamp>,
}

// ponytail: the key ignores `id`, so a container restarted inside cAdvisor's housekeeping window
// (old and new cgroup both listed) sums both and spikes the rate for one tick; key by `id` if it matters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContainerKey {
    namespace: String,
    pod: String,
    container: String,
}

#[derive(Default)]
struct Accumulated {
    read_bytes: u64,
    write_bytes: u64,
    sampled_at: Option<jiff::Timestamp>,
}

impl Accumulated {
    fn add(&mut self, line: &DiskIoLine<'_>) {
        let total = match line.direction {
            Direction::Read => &mut self.read_bytes,
            Direction::Write => &mut self.write_bytes,
        };
        *total = total.saturating_add(line.value);
        self.sampled_at = self.sampled_at.max(line.at);
    }

    fn counters(&self) -> DiskIoCounters {
        DiskIoCounters {
            sampled_at: self.sampled_at,
            read_bytes: self.read_bytes,
            write_bytes: self.write_bytes,
        }
    }
}

/// Collects the counters of one cAdvisor body, line by line.
#[derive(Default)]
pub(crate) struct DiskIoBuilder {
    node: Option<Accumulated>,
    containers: BTreeMap<ContainerKey, Accumulated>,
}

impl DiskIoBuilder {
    pub(crate) fn push_line(&mut self, line: &str) {
        let Some(line) = disk_io_line(line) else {
            return;
        };
        if line.id.as_deref() == Some("/") {
            self.node.get_or_insert_default().add(&line);
            return;
        }
        let (Some(namespace), Some(pod), Some(container)) =
            (&line.namespace, &line.pod, &line.container)
        else {
            return;
        };
        // The pause container is `POD`; an empty container is the pod cgroup itself.
        if namespace.is_empty() || pod.is_empty() || container.is_empty() || container == "POD" {
            return;
        }
        let key = ContainerKey {
            namespace: namespace.to_string(),
            pod: pod.to_string(),
            container: container.to_string(),
        };
        self.containers.entry(key).or_default().add(&line);
    }

    pub(crate) fn finish(self) -> DiskIoSample {
        DiskIoSample {
            node: self.node.as_ref().map(Accumulated::counters),
            containers: self
                .containers
                .into_iter()
                .map(|(key, accumulated)| ContainerDiskIo {
                    counters: accumulated.counters(),
                    namespace: key.namespace,
                    pod: key.pod,
                    container: key.container,
                })
                .collect(),
        }
    }
}

/// `None` for any other family and for a malformed line.
fn disk_io_line(line: &str) -> Option<DiskIoLine<'_>> {
    let (direction, mut rest) = if let Some(rest) = line.strip_prefix(READS_PREFIX) {
        (Direction::Read, rest)
    } else {
        (Direction::Write, line.strip_prefix(WRITES_PREFIX)?)
    };
    let mut parsed = DiskIoLine {
        direction,
        id: None,
        namespace: None,
        pod: None,
        container: None,
        value: 0,
        at: None,
    };
    loop {
        if let Some(after) = rest.strip_prefix('}') {
            rest = after;
            break;
        }
        let (key, after_key) = rest.split_once("=\"")?;
        let (raw, after_value) = split_label_value(after_key)?;
        let slot = match key {
            "id" => Some(&mut parsed.id),
            "namespace" => Some(&mut parsed.namespace),
            "pod" => Some(&mut parsed.pod),
            "container" => Some(&mut parsed.container),
            _ => None,
        };
        if let Some(slot) = slot {
            *slot = Some(unescape(raw));
        }
        // A trailing comma is allowed.
        rest = after_value.strip_prefix(',').unwrap_or(after_value);
        if !rest.starts_with('}') && rest.len() == after_value.len() {
            return None;
        }
    }
    let mut fields = rest.split_whitespace();
    let value: f64 = fields.next()?.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // Truncation is intended: counters are whole bytes.
    parsed.value = value as u64;
    if let Some(millis) = fields.next() {
        parsed.at = millis
            .parse::<i64>()
            .ok()
            .and_then(|millis| jiff::Timestamp::from_millisecond(millis).ok());
    }
    if fields.next().is_some() {
        return None;
    }
    Some(parsed)
}

/// Splits `text` after a label value that starts right after its opening quote: the raw
/// value up to the closing quote (escapes kept) and what follows the quote.
fn split_label_value(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            // Both are ASCII, so every index here is a char boundary.
            b'\\' => index += 2,
            b'"' => return Some((&text[..index], &text[index + 1..])),
            _ => index += 1,
        }
    }
    None
}

/// Resolves `\\`, `\"`, and `\n`; any other escape is kept as written.
fn unescape(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }
    let mut text = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => text.push('\n'),
            Some(escaped @ ('\\' | '"')) => text.push(escaped),
            Some(other) => {
                text.push('\\');
                text.push(other);
            }
            None => text.push('\\'),
        }
    }
    Cow::Owned(text)
}

#[cfg(test)]
#[path = "cadvisor_text_tests.rs"]
mod cadvisor_text_tests;
