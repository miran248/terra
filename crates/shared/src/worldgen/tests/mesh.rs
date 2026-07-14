use super::*;

#[test]
fn mesh_is_watertight() {
    // Every fall-through bug is a crack: an edge used by only one triangle
    // is a hole the player can drop through. On a closed surface each edge
    // is shared by EXACTLY two faces, and every vertex fan is a full ring.
    // Assert both on the baked mesh (the collider is built from it).
    let state = run_state(1337, |_| {});
    let grid = &state.grid;

    // (a) each undirected edge belongs to exactly two faces.
    let mut edge_faces: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for face_index in 0..grid.face_count() {
        let idx = grid.face_cells(FaceId::new(face_index)).map(CellId::index);
        for k in 0..3 {
            let (a, b) = (idx[k], idx[(k + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            *edge_faces.entry(key).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &edge_faces {
        assert_eq!(n, 2, "edge ({a},{b}) shared by {n} faces (not 2) — a crack");
    }

    // (b) every cell's faces form ONE closed fan: walking face→face
    // across shared edges visits all of them and returns. A cell whose
    // fan splits is a pinhole even if each edge is shared twice.
    for v in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(v)
            .expect("cell index from topology range");
        let faces = grid.topology.cell_faces(cell);
        let n = faces.len();
        assert!((5..=6).contains(&n), "cell {v} has {n} faces");
        let mut seen = vec![false; n];
        let mut stack = vec![0usize];
        seen[0] = true;
        let mut count = 1;
        while let Some(i) = stack.pop() {
            for j in 0..n {
                if seen[j] {
                    continue;
                }
                // Adjacent in the fan iff they share an edge through v
                // (two common vertices).
                let face_index = grid.face_cells(faces[i]).map(CellId::index);
                let fj = grid.face_cells(faces[j]).map(CellId::index);
                let shared = face_index.iter().filter(|x| fj.contains(x)).count();
                if shared == 2 {
                    seen[j] = true;
                    count += 1;
                    stack.push(j);
                }
            }
        }
        assert_eq!(count, n, "cell {v} fan is not one closed ring — a pinhole");
    }
}
