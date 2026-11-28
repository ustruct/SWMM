#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub bit: i32,
    pub left: u16,
    pub right: u16,
}

fn bit(key: &[u8], n: i32) -> usize {
    if n < 0 {
        return 0;
    }
    let byte = (n / 8) as usize;
    if byte >= key.len() {
        return 0;
    }
    ((key[key.len() - 1 - byte] >> (n % 8)) & 1) as usize
}

fn first_difference(a: &[u8], b: &[u8]) -> i32 {
    let bits = 8 * a.len().max(b.len()) as i32;
    (0..bits).find(|&n| bit(a, n) != bit(b, n)).unwrap_or(-1)
}

pub fn build(keys: &[&str]) -> Vec<Node> {
    let mut nodes = vec![Node { bit: -1, left: 0, right: 0 }];
    let mut names: Vec<&[u8]> = vec![b""];
    let child = |n: &Node, side: usize| if side == 0 { n.left } else { n.right };
    for key in keys {
        let k = key.as_bytes();
        let mut prev = 0usize;
        let mut cur = nodes[0].left as usize;
        while nodes[cur].bit > nodes[prev].bit {
            prev = cur;
            cur = child(&nodes[cur], bit(k, nodes[cur].bit)) as usize;
        }
        let b = first_difference(k, names[cur]);
        let mut parent = 0usize;
        let mut cur = nodes[0].left as usize;
        while nodes[cur].bit > nodes[parent].bit && nodes[cur].bit < b {
            parent = cur;
            cur = child(&nodes[cur], bit(k, nodes[cur].bit)) as usize;
        }
        let me = nodes.len() as u16;
        let mut node = Node { bit: b, left: 0, right: 0 };
        if bit(k, b) == 1 {
            node.right = me;
            node.left = cur as u16;
        } else {
            node.left = me;
            node.right = cur as u16;
        }
        nodes.push(node);
        names.push(k);
        let side = if parent == 0 { 0 } else { bit(k, nodes[parent].bit) };
        if side == 0 {
            nodes[parent].left = me;
        } else {
            nodes[parent].right = me;
        }
    }
    nodes
}
