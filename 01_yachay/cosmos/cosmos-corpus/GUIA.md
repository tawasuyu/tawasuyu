# Cómo generar el corpus de interpretación

Esta guía dice **exactamente** qué hacer, a mano, para construir el
corpus que `cosmobiologia` usará para interpretar cartas sin que ninguna
IA invente una palabra.

## Qué es (y qué NO es) el corpus

El corpus **no es un set de reglas matemáticas**. No "calcula" la
interpretación. Las reglas —qué planeta en qué signo, qué aspecto con
qué orbe— las computa el motor astronómico. El corpus es la **biblioteca
de evidencia**: fragmentos de texto —de los libros y de tu propia
escritura— recortados y **etiquetados** con la combinación exacta que
describen.

En runtime, las combinaciones de una carta hacen un **JOIN** contra el
corpus y traen los textos, citados y con fuente. La síntesis (tejerlos
en un párrafo continuo) es una capa posterior; el corpus solo
**almacena** y **recupera**.

La contradicción no se promedia. Un Marte hiperdisciplinado en el
trabajo y disperso en la soledad **no** se colapsa a "medio
disciplinado": cada fuerza vive intacta en su **dominio** vivencial. Por
eso el corpus rebana la carta en tajadas (`Vital`, `Social`, `Psiquico`)
— como ver un cuerpo en cortes tomográficos.

## El formato

Un archivo `.ron`. Mira `ejemplo.ron` en esta misma carpeta: es una
plantilla cargable y comentada. Tiene dos secciones, `arquetipos` y
`pasajes`.

### El "código de barras" de una combinación

Cada pasaje se etiqueta con una clave-cadena:

| Tipo | Sintaxis | Ejemplo |
|---|---|---|
| Planeta en signo | `planeta·signo` o `planeta/signo` | `mars·virgo`, `mars/virgo` |
| Planeta en casa | `planeta@cN` | `mars@c6` |
| Aspecto entre dos planetas | `a kind b` (tres palabras) | `mars square saturn` |

Reglas de los identificadores:

- minúscula, ASCII, **una sola palabra** (usa `_`: `north_node`);
- usa siempre el **mismo** nombre — `mars`, no `Marte` aquí y `mars`
  allá, o el JOIN no engancha;
- en un aspecto el orden da igual: `mars square saturn` y
  `saturn square mars` quedan como la misma clave.

## Los pasos

### Paso 1 — Crea tu archivo

```sh
cd 01_yachay/cosmos/cosmos-corpus
cp ejemplo.ron corpus.ron
```

Trabaja sobre `corpus.ron`. Borra los tres pasajes-plantilla cuando
tengas los tuyos.

### Paso 2 — (Opcional, recomendado) Escribe la ontología

En la sección `arquetipos`, una entrada por cada planeta, signo, casa y
aspecto que uses. Cada una lleva un `perfil`: un mapa de **dimensiones
psicológicas** —las nombras tú— con un peso en `[-1.0, 1.0]`.

```ron
(
    nombre: "mars",
    tipo: planeta,          // planeta | signo | casa | aspecto
    perfil: {
        "accion": 0.9,
        "deseo": 0.7,
    },
),
```

Esto **no es obligatorio para el JOIN** (el JOIN solo usa `pasajes`),
pero es la base para, más adelante, deducir el perfil de una combinación
que no llegaste a escribir. Si recién empiezas, puedes dejar
`arquetipos: []` y volver luego.

### Paso 3 — Cosecha los pasajes

Esta es la carne. Una entrada en `pasajes` por cada fragmento de
interpretación:

```ron
(
    combinacion: "mars·virgo",
    texto: "Cita literal, corta, del libro — o tu propia redacción.",
    fuente: "Autor, Título de la obra, p. 123",
),
```

Dos formas de avanzar; elige una:

- **Por fuente** — tomas un libro y lo vacías combinación por
  combinación. Bueno para cubrir un autor entero de forma pareja.
- **Por carta** — tomas la carta que estás leyendo *ahora*, listas sus
  combinaciones y solo escribes esas. Bueno para tener algo útil ya, sin
  esperar a "terminar" el corpus (que nunca termina).

Recomendado: empieza **por carta**. El corpus crece con cada consulta
real.

### Paso 4 — Cuida la fuente y el derecho de autor

- Cita **corto** y **textual**, y **atribuye siempre** (autor, obra,
  página). Fragmentos breves con cita son uso legítimo.
- No copies capítulos enteros. Si quieres volcar una idea larga,
  **reescríbela con tus palabras** y pon `fuente: "propio"`.
- Convención reservada: `fuente: "deducido"` queda para perfiles
  compuestos por código a futuro, no para texto de libro.

### Paso 5 — Acota el dominio cuando el texto lo pida

Si un pasaje describe la combinación **solo en un plano** de la vida,
márcalo:

```ron
(
    combinacion: "mars square saturn",
    texto: "...",
    fuente: "...",
    dominio: Some(psiquico),   // vital | social | psiquico
),
```

Sin `dominio`, el pasaje aplica al dominio que le toque por la posición
del planeta en la carta. Con `dominio`, lo fuerzas. Úsalo poco: solo
cuando el autor habla de un plano concreto.

### Paso 6 — Valida el archivo

```sh
cargo test -p cosmos-corpus
```

Si tu RON tiene un error de sintaxis, el test `ejemplo_ron_carga`
te marca el formato correcto; para validar `corpus.ron` directamente,
cárgalo desde un binario o un test propio con `Corpus::desde_ron`.

### Paso 7 — Busca los huecos

Con la carta cargada, `Corpus::huecos(&combinaciones)` devuelve las
combinaciones de esa carta que **no tienen ni un pasaje**. Esa lista es,
literalmente, tu cola de trabajo: lo que falta escribir.

## Cuánto es "suficiente"

El universo completo es grande (≈10 planetas × 12 signos = 120, otras
120 planeta-en-casa, y los aspectos). No lo persigas. El 80 % del valor
sale del 20 %: las combinaciones que de verdad aparecen en las cartas
que lees. Empieza con una carta, deja que `huecos` te guíe, y el corpus
se llena solo, consulta a consulta.
