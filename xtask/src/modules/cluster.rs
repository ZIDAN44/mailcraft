//! Groups in a weighted, undirected graph: Louvain's modularity optimisation (Blondel et al.,
//! 2008), each group then split into its connected parts, the guarantee Leiden adds (Traag et
//! al., 2019) and the one that matters for a folder: a group whose files don't touch isn't one.
//! Deterministic: nodes are visited in order, ties keep the lower group.

/// An undirected graph on nodes `0..n`: `(a, b, weight)`, each pair once.
pub type Edges = Vec<(usize, usize, f64)>;

/// Each node's group, numbered from 0 by the group's smallest node.
pub fn groups(n: usize, edges: &Edges) -> Vec<usize> {
    let mut of: Vec<usize> = (0..n).collect();
    let mut level_n = n;
    let mut level_edges = edges.clone();
    loop {
        let (moved, community) = one_level(level_n, &level_edges);
        if !moved {
            break;
        }
        let (renumbered, count) = renumber(&community);
        for g in of.iter_mut() {
            *g = renumbered.get(*g).copied().unwrap_or(*g);
        }
        // The next level: a node per group, the weights between groups summed (within a group:
        // a loop, kept so its weight still counts).
        let mut summed: std::collections::BTreeMap<(usize, usize), f64> = std::collections::BTreeMap::new();
        for &(a, b, w) in &level_edges {
            let (x, y) = (renumbered.get(a).copied().unwrap_or(a), renumbered.get(b).copied().unwrap_or(b));
            *summed.entry((x.min(y), x.max(y))).or_default() += w;
        }
        level_edges = summed.into_iter().map(|((a, b), w)| (a, b, w)).collect();
        if count == level_n {
            break;
        }
        level_n = count;
    }
    connected_parts(n, edges, &of)
}

/// One pass of moving nodes to the neighbouring group that raises modularity most, until none
/// does. Whether anything moved, and each node's group.
fn one_level(n: usize, edges: &Edges) -> (bool, Vec<usize>) {
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut degree = vec![0.0; n];
    let mut total = 0.0;
    for &(a, b, w) in edges {
        if a >= n || b >= n {
            continue;
        }
        if a == b {
            degree[a] += 2.0 * w;
        } else {
            adj[a].push((b, w));
            adj[b].push((a, w));
            degree[a] += w;
            degree[b] += w;
        }
        total += 2.0 * w;
    }
    let mut community: Vec<usize> = (0..n).collect();
    let mut tot: Vec<f64> = degree.clone();
    if total <= 0.0 {
        return (false, community);
    }
    let mut moved_any = false;
    for _round in 0..100 {
        let mut moved = false;
        for i in 0..n {
            let here = community[i];
            tot[here] -= degree[i];
            let mut to: std::collections::BTreeMap<usize, f64> = std::collections::BTreeMap::new();
            for &(j, w) in &adj[i] {
                *to.entry(community[j]).or_default() += w;
            }
            let gain = |c: usize, k_in: f64| k_in - tot[c] * degree[i] / total;
            let mut best = (here, gain(here, to.get(&here).copied().unwrap_or(0.0)));
            for (&c, &k_in) in &to {
                let g = gain(c, k_in);
                if g > best.1 + 1e-12 {
                    best = (c, g);
                }
            }
            community[i] = best.0;
            tot[best.0] += degree[i];
            if best.0 != here {
                moved = true;
                moved_any = true;
            }
        }
        if !moved {
            break;
        }
    }
    (moved_any, community)
}

/// Groups numbered `0..count` in the order their first node comes.
fn renumber(community: &[usize]) -> (Vec<usize>, usize) {
    let mut map = std::collections::BTreeMap::new();
    let out = community
        .iter()
        .map(|c| {
            let next = map.len();
            *map.entry(*c).or_insert(next)
        })
        .collect();
    (out, map.len())
}

/// Each group split into the parts of it that are connected by its own edges.
fn connected_parts(n: usize, edges: &Edges, of: &[usize]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for &(a, b, _) in edges {
        if a < n && b < n && a != b && of.get(a) == of.get(b) {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    renumber(&roots).0
}

/// How well `of` groups the graph (Newman's modularity): the share of weight inside groups,
/// less what chance would put there. 0 when there's no weight.
pub fn modularity(n: usize, edges: &Edges, of: &[usize]) -> f64 {
    let groups = of.iter().copied().max().map_or(0, |m| m + 1);
    let (mut inside, mut tot) = (vec![0.0; groups], vec![0.0; groups]);
    let mut total = 0.0;
    for &(a, b, w) in edges {
        let (Some(&ga), Some(&gb)) = (of.get(a), of.get(b)) else { continue };
        if a >= n || b >= n {
            continue;
        }
        total += 2.0 * w;
        tot[ga] += w;
        tot[gb] += w;
        if ga == gb {
            inside[ga] += 2.0 * w;
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    (0..groups).map(|g| inside[g] / total - (tot[g] / total).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles joined by one light edge: two groups, scored well; a group that isn't
    /// connected is split.
    #[test]
    fn two_triangles_are_two_groups() {
        let edges: Edges = vec![(0, 1, 1.0), (1, 2, 1.0), (0, 2, 1.0), (3, 4, 1.0), (4, 5, 1.0), (3, 5, 1.0), (2, 3, 0.1)];
        let of = groups(6, &edges);
        assert_eq!(of, [0, 0, 0, 1, 1, 1]);
        assert!(modularity(6, &edges, &of) > 0.4);
        assert!(modularity(6, &edges, &[0, 0, 0, 0, 0, 0]).abs() < 1e-9);
        assert_eq!(connected_parts(4, &vec![(0, 1, 1.0), (2, 3, 1.0)], &[0, 0, 0, 0]), [0, 0, 1, 1]);
        assert_eq!(groups(3, &Vec::new()), [0, 1, 2], "no edges: each alone");
    }
}
