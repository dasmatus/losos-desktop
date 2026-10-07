//! Just enough of the DNS wire format (RFC 1035) to read a query's name and
//! answer it for a blocked one. Everything else is forwarded byte for byte,
//! so nothing here has to understand a response.

/// The question of a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// The name asked about, lowercase, without the trailing dot.
    pub name: String,
    /// The record type (1 is A, 28 is AAAA).
    pub qtype: u16,
    /// Where the question ends in the packet.
    end: usize,
}

pub const TYPE_A: u16 = 1;
pub const TYPE_AAAA: u16 = 28;
const HEADER: usize = 12;

/// The first question of a query, or `None` for a packet that is not a
/// standard query with one.
pub fn question(packet: &[u8]) -> Option<Question> {
    if packet.len() < HEADER {
        return None;
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    let is_response = flags & 0x8000 != 0;
    let opcode = (flags >> 11) & 0xf;
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if is_response || opcode != 0 || qdcount != 1 {
        return None;
    }
    let mut at = HEADER;
    let mut labels: Vec<String> = Vec::new();
    loop {
        let len = *packet.get(at)? as usize;
        at += 1;
        if len == 0 {
            break;
        }
        // A query's name is never compressed; a pointer here is malformed.
        if len & 0xc0 != 0 {
            return None;
        }
        let label = packet.get(at..at + len)?;
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        at += len;
    }
    let qtype = u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]);
    // Class follows the type.
    packet.get(at + 3)?;
    Some(Question {
        name: labels.join("."),
        qtype,
        end: at + 4,
    })
}

/// The answer to a blocked query: an address nothing listens on for A and
/// AAAA (0.0.0.0 and ::, as Pi-hole and AdGuard Home answer), so a program
/// fails at once instead of retrying, and no records at all for any other
/// type. Not NXDOMAIN: that is cached for the whole name and every type,
/// and some programs read it as "offline".
pub fn blocked(query: &[u8], question: &Question) -> Vec<u8> {
    let mut out = Vec::with_capacity(question.end + 28);
    // Header: the query's id; QR, the query's RD, RA; one question.
    out.extend_from_slice(&query[0..2]);
    let rd = query[2] & 0x01;
    out.push(0x80 | rd);
    out.push(0x80);
    out.extend_from_slice(&1u16.to_be_bytes());
    let rdata: &[u8] = match question.qtype {
        TYPE_A => &[0; 4],
        TYPE_AAAA => &[0; 16],
        _ => &[],
    };
    let answers: u16 = if rdata.is_empty() { 0 } else { 1 };
    out.extend_from_slice(&answers.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    // The question, as asked.
    out.extend_from_slice(&query[HEADER..question.end]);
    if answers == 1 {
        // A pointer to the question's name, the type and class asked, and
        // a short TTL so a list update shows within minutes.
        out.extend_from_slice(&[0xc0, HEADER as u8]);
        out.extend_from_slice(&question.qtype.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&300u32.to_be_bytes());
        out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(rdata);
    }
    out
}

/// A SERVFAIL for a query that could not be forwarded, so the stub retries
/// rather than waiting out its own timeout.
pub fn servfail(query: &[u8]) -> Option<Vec<u8>> {
    if query.len() < HEADER {
        return None;
    }
    let mut out = query.to_vec();
    out[2] = 0x80 | (query[2] & 0x79);
    out[3] = 0x80 | 2;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(name: &str, qtype: u16) -> Vec<u8> {
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&1u16.to_be_bytes());
        q
    }

    #[test]
    fn reads_the_question() {
        let q = query("Ads.Example.com", TYPE_A);
        let question = question(&q).unwrap();
        assert_eq!(question.name, "ads.example.com");
        assert_eq!(question.qtype, TYPE_A);
    }

    #[test]
    fn answers_a_with_zero() {
        let q = query("ads.example.com", TYPE_A);
        let answer = blocked(&q, &question(&q).unwrap());
        assert_eq!(&answer[0..2], &[0x12, 0x34]);
        assert_eq!(answer[2] & 0x80, 0x80);
        assert_eq!(answer[3] & 0x0f, 0);
        assert_eq!(u16::from_be_bytes([answer[6], answer[7]]), 1);
        assert_eq!(&answer[answer.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn answers_other_types_with_nothing() {
        let q = query("ads.example.com", 16);
        let answer = blocked(&q, &question(&q).unwrap());
        assert_eq!(u16::from_be_bytes([answer[6], answer[7]]), 0);
        assert_eq!(answer.len(), q.len());
    }

    #[test]
    fn ignores_responses_and_garbage() {
        let mut q = query("example.com", TYPE_A);
        q[2] |= 0x80;
        assert_eq!(question(&q), None);
        assert_eq!(question(&[1, 2, 3]), None);
        assert_eq!(question(&query("example.com", TYPE_A)[..20]), None);
    }
}
