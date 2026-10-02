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
				case Closed | Destroyed:
					running = false;
				case Resized(width, height):
					trace('resized to ${width}x${height}');
				case CursorMoved(x, y, device):
					trace('cursor $device at $x, $y');
				case MouseInput(state, button, device):
					trace('mouse $device: $button $state');
				case MouseWheel(LineDelta(x, y), phase, device):
					trace('wheel $device: $x, $y lines');
				case MouseWheel(PixelDelta(x, y), phase, device):
					trace('wheel $device: $x, $y pixels');
				case KeyboardInput(device, Input(physical, logical, text, location, state, repeat, _), synthetic):
					trace('key $physical $logical $text $location $state repeat=$repeat synthetic=$synthetic');
					switch [physical, state] {
						case [Code(KeyCode.Escape), Pressed]: running = false;
						case _:
					}
				case ModifiersChanged(State(shift, control, alt, superKey, leftShift, _, _, _, _, _, _, _)):
					trace('modifiers shift=$shift ($leftShift) control=$control alt=$alt super=$superKey');
				case Ime(Commit(text)):
					trace('typed "$text"');
				case DroppedFile(Utf8(path)):
					trace('dropped $path');
				case ScaleFactorChanged(scale):
					trace('scale $scale');
				case RedrawRequested:
					trace("redraw");
				case other:
					trace(other);
			}
		}
		window.close();
	}
}
