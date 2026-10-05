# vasak-health-service

El servicio de salud de VasakOS. Hoy mide una sola cosa —el **tiempo de
pantalla por aplicación**—, pero es un servicio aparte y se llama «salud» a
propósito: mañana puede medir más cosas de bienestar sin mudar nada.

Cuenta cuánto tiempo estuvo enfocada cada aplicación, **por día y por hora**, y
le pone a cada una su **categoría** (redes, desarrollo, un juego…, sacada de su
`.desktop`). Con eso el escritorio y Configuración arman sus informes.

## Cómo está partido

Un espacio de trabajo de Cargo con dos miembros, por el mismo motivo que
`vasak-connect`: que el escritorio y Configuración puedan depender del contrato
sin arrastrar nada de lo que el servicio necesita para su trabajo.

- **`protocol/`** — el contrato D-Bus: las direcciones del bus, los tipos que
  viajan (`AppUsage`, `DayUsage`, `ScreenTimeReport`) y el proxy. Es lo único
  que comparten los tres lados.
- **`daemon/`** — el servicio: la contabilidad (`tracker`), el almacén por día
  (`store`), las categorías (`category`), el interruptor (`config`) y la
  interfaz D-Bus (`service`).

## Quién mide y quién pregunta

El **escritorio** es el único que habla con el compositor (la ventana enfocada,
por el socket de Wayfire) y con la inactividad (`ext-idle-notify-v1`). Ya tiene
esas dos conexiones para dibujar el panel, así que es él quien **empuja las
muestras** a este servicio: qué aplicación está enfocada, si la pantalla está
bloqueada, cuándo empezó y terminó la inactividad.

Este servicio hace el resto: la contabilidad (descontar la inactividad, repartir
lo que cruza la medianoche y cada hora, no contar los saltos del reloj),
guardarla, ponerle categoría y devolverla cuando el escritorio o Configuración la
consultan. Un segundo proceso abriendo sus propias conexiones a Wayland sería
otra conexión, otro permiso y más trabajo para lo mismo.

## Dónde se guarda, y la privacidad

Un archivo JSON por día en
`$XDG_DATA_HOME/vasak-health-service/screen-time/AAAA-MM-DD.json`. **Nada sale de
la máquina**: los archivos son del usuario y nadie más los lee. Se apaga con
`screen_time.enabled: false` en `vasak.conf` —lo escribe el interruptor del
tablero y el de Configuración— y se borra con «Borrar historial».

La primera vez, si todavía no hay nada guardado, importa lo que hubiera dejado la
versión que vivía dentro del escritorio
(`$XDG_DATA_HOME/vasak-desktop/screen-time/`), para no perder la historia.

## El bus

Corre en el bus de **sesión**, como el usuario con la sesión abierta.

- Nombre: `ar.net.vasak.os.Health`
- Ruta: `/ar/net/vasak/os/Health`

Lo arranca systemd (`vasak-health-service.service`, parte de
`graphical-session.target`) y también se puede activar por D-Bus si alguien lo
consulta antes de que la sesión gráfica haya llegado tan lejos.

## Compilar y probar

```bash
cargo build
cargo test
```
