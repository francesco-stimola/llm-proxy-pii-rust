//! **The tool-result fixture (M11-R55): a real agent turn whose text is command output.**
//!
//! PHONE-OM runs over the M7 turn — instruction boilerplate, tool schemas and one short user
//! message — and that turn has a **shape**: `grep -cE "[0-9]{2,3}( ){2,4}[0-9]{2,4}"` over it
//! is `0`, and it holds no IPv4 address at all. So it was structurally unable to see either
//! class the round-14 separator widening admitted, and it stayed green through both. *A corpus
//! has a shape, and that shape is a blind spot* (M4-R13) — landing, once again, on the guard
//! written to defeat exactly that.
//!
//! This is the missing shape, and nothing about it is adversarial: it is what a coding agent
//! reads all day. `ls -l`, `df -h`, a `psql` result, `journalctl` lines, a `netstat` table, and
//! the `tool_use.input` of an `ssh` command. Column-aligned digits and dotted quads are the
//! **default** rendering of that text, not a corner of it.
//!
//! **Kept separate from `m7_turn.rs` on purpose.** That fixture measures the *cost model* — one
//! big system field, many small schema fields, one tiny user message — and splicing command
//! output into it would change what every latency number there means. Two fixtures, two claims:
//! the M7 turn says what an *instruction-shaped* request costs, this one says what an
//! *output-shaped* one is masked into.
//!
//! **Its expectation is a measured number, not a zero.** M11-R55 is the decision to accept this
//! over-mask and publish it; a fixture asserting zero here would be asserting the opposite of
//! what the product does. What the guard is for is that the number cannot **move** without
//! somebody changing it on purpose — see `tests/phone_overmask.rs`.

// The consumers use different subsets, and an unused item in the other crate is not a defect.
#![allow(dead_code)]

pub struct Field {
    pub name: String,
    pub text: String,
}

fn field(name: &str, text: &str) -> Field {
    Field {
        name: name.to_string(),
        text: text.to_string(),
    }
}

/// A turn of six fields: four `tool_result`s, one `tool_use.input`, one user message.
///
/// Every number in it is a real quantity of its kind — sizes, inodes, PIDs, ports, row counts,
/// octets — and **none of them is a phone number**. Any `Phone` span found here is an over-mask
/// by construction, which is what makes the fixture able to carry a precision claim at all.
pub fn tool_output_turn() -> Vec<Field> {
    vec![
        field(
            "tool_result[ls]",
            "$ ls -l /var/lib/app/releases\n\
             total 4820\n\
             drwxr-xr-x  2 deploy deploy    4096 Jul 14 09:12 2026.07.14-a91f2\n\
             -rw-r--r--  1 deploy deploy  128394 Jul 14 09:12 bundle.tar.gz\n\
             -rw-r--r--  1 deploy deploy   20480 Jul 15 11:03 manifest.json\n\
             -rwxr-xr-x  1 deploy deploy 3149216 Aug 02 18:44 llm-proxy-pii-rust\n\
             -rw-r--r--  1 deploy deploy    1024 Aug 02 18:44 checksum.txt\n",
        ),
        field(
            "tool_result[df]",
            "$ df -h\n\
             Filesystem      Size  Used Avail Use% Mounted on\n\
             /dev/nvme0n1p2  916G  412G  458G  48% /\n\
             /dev/nvme0n1p1  511M   62M  450M  13% /boot/efi\n\
             tmpfs            32G  1.1G   31G   4% /dev/shm\n\
             \n\
             $ free -m\n\
             total        used        free      shared\n\
             32105       11842       12903        1204\n\
             16384        4096        8192         512\n",
        ),
        field(
            "tool_result[psql]",
            "$ psql -c 'select id, qty, price, warehouse from stock order by id limit 6'\n\
             id    qty   price  warehouse\n\
             101   250   1999   140\n\
             205   310   2499   140\n\
             318   120   3499   205\n\
             427    90   1299   318\n\
             512   105   205    301\n\
             913   107   207    307\n\
             (6 rows)\n",
        ),
        field(
            "tool_result[journal]",
            "$ journalctl -u llm-proxy --since '10 min ago' | tail\n\
             upstream 62.30.40.50 refused the connection after 3 retries\n\
             upstream 170.75.154.131 answered 200 in 412 ms\n\
             peer 10.55.120.7 timed out, falling back to 192.168.14.203\n\
             health check on 172.16.31.9 ok, latency 42 ms\n\
             listening on 0.0.0.0:8080, metrics on 127.0.0.1:9090\n\
             rotated log at 2026-08-02, next rotation 2026-09-01\n",
        ),
        field(
            "tool_use[ssh].input",
            "{\"command\": \"ssh deploy@62.30.40.50 -p 22 -- 'grep -c 512 105 205 /var/log/app.log'\", \
             \"timeout\": 30000}",
        ),
        field(
            "user",
            "The deploy box is unreachable again. Check the psql output above and tell me which \
             warehouse row is wrong; my contact is mario.rossi@example.com.",
        ),
    ]
}
