import window.Gamepad;
import window.GamepadEvent;

/** Controller events and a reusable snapshot, through the generated window API. */
class Main {
    static function main():Void {
        if(!Gamepad.available()) { Sys.println(Gamepad.error()); return; }
        Sys.println('Connected controllers: '+Gamepad.count());
        var state=haxe.io.Bytes.alloc(176);
        var pad=Gamepad.at(0);
        for(frame in 0...600) {
            var event=Gamepad.poll();
            while(event!=None) { Sys.println(Std.string(event)); event=Gamepad.poll(); }
            if(!pad.valid() && Gamepad.count()>0) pad=Gamepad.at(0);
            if(pad.valid()) {
                if(pad.snapshot(state) && frame%60==0)
                    Sys.println(pad.name()+': left stick '+state.getDouble(136)+', '+state.getDouble(144));
            }
            Sys.sleep(1.0/60.0);
        }
    }
}
