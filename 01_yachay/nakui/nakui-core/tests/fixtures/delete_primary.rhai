// Deletes its primary input. The kernel must:
//   - accept the Delete op (token = "Caja", in writes)
//   - skip the per-input KCL post-check (entity no longer exists)
//   - allow apply to remove the record cleanly
[
    #{ op: "delete", entity: "Caja", id: input.ids.caja },
]
