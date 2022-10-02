# Read_Jeelink

This is an experimental crate to experiment with reading data from a serial port.
In this case, the device is a [Jeelink v3c](https://www.digitalsmarties.net/products/jeelink) flashed with the [LaCrosseITPlusReader Arduino sketch from FHEM](https://svn.fhem.de/trac/browser/trunk/fhem/contrib/arduino).
The sketch has been changed such that the annoying blue LED is permanently switched off and the data rate of RFM #1 is set to 9579 bps which is what my home sensors (TX35DTH-IT) require.
The details about the message format can be found in the Arduino sketch.

## Goal of this project
Develop into an asynchronous reader to be used in a [Tokio](https://tokio.rs/) based home automation server.