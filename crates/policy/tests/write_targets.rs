//! `write_targets` reads the words a command line writes through
//! (`bash`): redirections it can prove, and the file arguments of the
//! commands that write. The reader is conservative, so a row here states
//! exactly which words a command's own writing names, never what a shell
//! would do with a variable or a `sh -c` string.

use aigentic_policy::write_targets;

#[test]
fn write_targets_names_the_words_a_command_writes() {
    let rows: [(&str, &[&str]); 32] = [
        // Redirections.
        ("echo x >> f", &["f"]),
        ("echo a > f > g", &["f", "g"]),
        ("echo x > 1", &["1"]),
        ("echo x >&2", &[]),
        ("echo x >&f", &["f"]),
        ("cat < f", &[]),
        ("echo x > /dev/null", &["/dev/null"]),
        ("cargo test && echo x > f", &["f"]),
        // Commands whose arguments are files.
        ("tee -a f g", &["f", "g"]),
        ("cp a f", &["f"]),
        ("cp a b f", &["f"]),
        ("cp -t d a", &["d"]),
        ("mv a f", &["f"]),
        ("install -m 644 a f", &["f"]),
        ("ln -s a f", &["f"]),
        ("touch f g", &["f", "g"]),
        ("mkdir -p f", &["f"]),
        ("rm -rf f g", &["f", "g"]),
        ("truncate -s 0 f", &["f"]),
        ("sed -i s/a/b/ f g", &["f", "g"]),
        ("sed -i -e s/a/b/ f", &["f"]),
        ("sed -i s/a/b/ -", &[]),
        ("sort -o f in", &["f"]),
        ("dd if=a of=f", &["f"]),
        ("find f -delete", &["f"]),
        ("find f -name x", &[]),
        ("find f -fprint g", &["g"]),
        ("sudo cp a f", &["f"]),
        ("nice -n 5 cp a f", &["f"]),
        ("echo x $(date > f)", &["f"]),
        // Commands that write nothing.
        ("cat f", &[]),
        ("ls -l", &[]),
    ];
    for (command, expected) in rows {
        assert_eq!(write_targets(command), expected, "{command}");
    }
}
