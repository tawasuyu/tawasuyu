// Cierra el período: marca la caja como cerrada.
[
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.caja, field: "estado" },
       value: "cerrada" },
]
