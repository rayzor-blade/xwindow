import window.*;

/**
	Opens a window and prints its events until it is asked to close. The same
	source builds against every runtime's generated `window` package.
**/
class Main {
	static function main() {
		var attributes = new WindowAttributes();
		attributes.title("xwindow events");
		attributes.width(800);
		attributes.height(600);
		var window = Window.open(attributes);
		if (!window.valid()) {
			trace("no window");
			return;
		}
		window.setImeAllowed(true);
		window.setImeCursorArea(20, 20, 400, 24);
		window.requestRedraw();
		trace('platform ${window.platform()}, ${window.width()}x${window.height()} at ${window.scaleFactor()}');
		var monitor = window.currentMonitor();
		if (monitor.valid())
			trace('on ${monitor.name()}: ${monitor.width()}x${monitor.height()}');

		var running = true;
		while (running) {
			switch (window.wait(0.5)) {
				case None:
				case CloseRequested | Destroyed:
					running = false;
				case Resized(width, height):
					trace('resized to ${width}x${height}');
				case CursorMoved(device, x, y):
					trace('cursor $device at $x, $y');
				case MouseInput(device, button, code, pressed):
					trace('mouse $device: $button ${pressed ? "down" : "up"}');
				case MouseWheel(device, unit, x, y, phase):
					trace('wheel $device: $x, $y $unit');
				case KeyboardInput(device, code, scancode, key, character, text, location, pressed, repeat, synthetic):
					trace('key $code ($key "$character") ${pressed ? "down" : "up"} text "$text"');
					if (code == KeyCode.Escape && pressed)
						running = false;
				case ModifiersChanged(shift, control, alt, superKey):
					trace('modifiers shift=$shift control=$control alt=$alt super=$superKey');
				case ImeCommit(text):
					trace('typed "$text"');
				case ScaleFactorChanged(scaleFactor):
					trace('scale $scaleFactor');
				case RedrawRequested:
					trace("redraw");
				case other:
					trace(other);
			}
		}
		window.close();
	}
}
