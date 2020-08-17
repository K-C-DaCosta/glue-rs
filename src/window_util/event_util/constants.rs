use nalgebra::Vector2;

//the whole point of this module is to provide a generic interface for events in the code
#[derive(Copy, Clone)]
pub enum EventKinds {
    Quit,
    Resize,
    MouseMove(Vector2<f32>),
    MouseDown(Vector2<f32>),
    MouseUp(Vector2<f32>),
    TouchMove(Vector2<f32>),
    TouchDown(Vector2<f32>),
    TouchUp(Vector2<f32>),
    KeyDown(KeyKinds),
    KeyUp(KeyKinds),
}

#[derive(Copy, Clone,PartialEq)]
#[allow(non_camel_case_types)]
pub enum KeyKinds {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    SUPER,
    L_SHIFT,
    R_SHIFT,
    L_CTRL,
    R_CTRL,
    TAB,
    CAPS_LOCK,
    PAGE_UP,
    PAGE_DOWN,
    HOME,
    END,
    DEL,
    INSERT,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    ESC,
    NUM_1,
    NUM_2,
    NUM_3,
    NUM_4,
    NUM_5,
    NUM_6,
    NUM_7,
    NUM_8,
    NUM_9,
    NUM_0,
    ARROW_LEFT,
    ARROW_UP,
    ARROW_RIGHT,
    ARROW_DOWN,
    BACKSPACE,
    ENTER,
    PLUS,
    MINUS,
}
