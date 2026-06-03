# Rectificador de hora — manual de uso

El **rectificador** estima la hora de nacimiento verdadera cuando la
registrada es incierta. Usa el método de **direcciones primarias** del
**Sistema GR (Germán Rosas)**: en la hora correcta, los eventos reales de
la vida del sujeto **coinciden** con la perfección de una dirección
primaria (el arco que la esfera celeste rota tras el nacimiento hasta que
un promisor alcanza la posición mundana de un significador).

La trigonometría esférica de esos arcos (método Placidus-mundano,
semi-arcos diurnos/nocturnos bajo el polo de cada cuerpo) la aporta
`eternal-astrology`; el rectificador es la capa de **optimización**: barre
las horas candidatas y minimiza el desajuste entre los eventos conocidos y
los arcos teóricos.

## Dónde está

Panel **«Rectificador de hora»**, en la categoría **Sistema** (engranaje)
del panel de herramientas.

## Flujo de trabajo

1. **Cargá la carta** del sujeto (la hora registrada/estimada es el punto
   de partida del barrido).

2. **Jog de hora** — los botones `-60 -10 -1 +1 +10 +60` corren la hora de
   nacimiento en minutos **sin tocar la carta guardada**. Sirve para
   explorar a mano: mirá cómo se mueven el Ascendente, el MC y las casas en
   la rueda mientras ajustás. `0` vuelve al offset cero.

3. **Eventos conocidos** — `+ evento` agrega un ancla; cada fila es la
   **edad del sujeto** (en años) cuando ocurrió un hecho fuerte y datable
   (matrimonio, mudanza, muerte de un padre, nacimiento de un hijo,
   accidente…). Ajustá con `-1 / +1` (años) y `-0.1 / +0.1` (≈ mes y
   medio). `quitar` borra la fila.

   Cuantos más eventos buenos cargues, más nítido el valle. Con uno solo,
   el barrido puede tener varios mínimos: usá 3–5.

4. **Rectificar** — corre el barrido de **dos pasadas** sobre ±2 h:
   - **gruesa**, minuto a minuto sobre toda la ventana (es la curva de
     perfil que se dibuja);
   - **fina**, segundo a segundo alrededor del mejor minuto (de ahí la
     precisión de segundo).

5. **Resultado** — se muestra el mejor offset (`+s`, su equivalente en
   `min s`) y el **error** del candidato (suma, en años, del desajuste de
   cada evento a su dirección primaria más cercana; **menor = mejor**).
   Debajo, la **curva de perfil**: el eje X es el offset y el **valle**
   (marcado con la línea de acento) es la hora rectificada.

6. **Aplicar al nacimiento** — escribe el mejor offset en la hora de la
   carta (`hour:minute:second`), marca la certeza como *exacta*, persiste
   la carta y recomputa. El jog vuelve a `0`.

## Lectura de la curva

- Un **valle único y profundo** → rectificación confiable.
- **Varios valles parecidos** → faltan anclas: agregá más eventos o usá
  eventos más separados en el tiempo.
- **Curva casi plana** → los eventos no discriminan la hora (poco rango de
  declinaciones tocadas); revisá las edades.

## Notas

- La clave arco↔año por defecto es **Naibod** (0°59′08.33″/año, estándar
  moderno). El motor también soporta **Ptolomeo** (1°/año); hoy el panel
  usa Naibod.
- La ventana del barrido es de **±2 h**. Si la hora registrada puede estar
  más lejos, conviene primero acercarse con el jog y luego rectificar.
- El jog y el barrido **no modifican la carta** hasta que tocás *Aplicar*.
- El motor (`cosmos-engine::rectificar` + `cosmos-render::gr` — los
  *triggers* GR de convergencia directo/converso) está disponible siempre
  que el feature `eternal-bridge` esté activo (lo está por defecto).
