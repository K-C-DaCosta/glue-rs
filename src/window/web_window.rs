use super::{event_util::*, FlufflError, TouchTracker, WindowManager};

use crate::audio::{init_audio_threads, FlufflAudioContext};

use crate::{console::*, *};

pub use js_sys;
pub use wasm_bindgen;
pub use wasm_bindgen::{prelude::*, JsCast};
pub use web_sys::*;

use std::{
    cell::{Cell, RefCell},
    mem,
    rc::Rc,
    sync::Arc,
};

use glow;
use glow::*;
use wasm_bindgen_futures::*;
use web_sys;

use super::{FlufflWindowConfigs, FlufflWindowPtr};

// Global variables that are only visible inside of this module.
// The use of global variables should be fine if there is no multithreading going on.
static mut GLOBAL_EVENT_QUEUE: Option<FlufflEvent> = None;
static mut GLOBAL_CANVAS_REF: Option<Rc<HtmlCanvasElement>> = None;
static mut IS_MOBILE: bool = false;

/// determines if desktop is mobile or not
fn determine_desktop_or_mobile() {
    let navigator = web_sys::window().unwrap().navigator();

    let mobile_platforms = [
        "Android",
        "webOS",
        "iPad",
        "iPhone",
        "iPod",
        "BlackBerry",
        "Windows Phone",
        "IEMobile",
    ];

    match navigator.user_agent() {
        Ok(agent_string) => {
            for platform in mobile_platforms.iter() {
                let has_match = agent_string
                    .to_lowercase()
                    .matches(platform.to_lowercase().as_str())
                    .next()
                    .is_some();

                if has_match {
                    unsafe {
                        IS_MOBILE = true;
                        break;
                    }
                }
            }
        }
        Err(_) => panic!("Failed to determine browser platform!"),
    }
}

fn is_mobile() -> bool {
    unsafe { IS_MOBILE }
}

fn init_global_event_queue() {
    unsafe { GLOBAL_EVENT_QUEUE = Some(FlufflEvent::new()) };
}

fn init_global_canvas(canvas_ptr: Rc<HtmlCanvasElement>) {
    unsafe {
        GLOBAL_CANVAS_REF = Some(canvas_ptr);
    }
}

fn get_canvas() -> Rc<HtmlCanvasElement> {
    unsafe {
        GLOBAL_CANVAS_REF
            .as_ref()
            .expect("global canvas pointer not initalized!")
            .clone()
    }
}

#[allow(dead_code)]
pub struct FlufflWindow {
    glue_event: Option<FlufflEvent>,
    gl: Arc<Box<Context>>,
    render_loop: Option<glow::RenderLoop>,
    window_width: u32,
    window_height: u32,
    audio_ctx: Arc<RefCell<FlufflAudioContext>>,
    canvas: Rc<HtmlCanvasElement>,
}

impl FlufflWindow {
    //moves renderloop out of glue_window and calls it
    fn get_render_loop(&mut self) -> Option<impl glow::HasRenderLoop> {
        self.render_loop.take()
    }

    /// Loops infinitely unless caller mutates `running` to false. Just executes the user defined taske over and over while also \
    /// doing basic matenence for events and the window
    /// # Arguments
    /// * `closure` is the user defined task. it exposes platform specific internals with `&mut Self`. \
    /// ```
    pub fn main_loop<F, T>(
        mut fluffr_window: Self,
        aux: T,
        core_loop: impl Fn(FlufflWindowPtr, Rc<Cell<bool>>, Rc<RefCell<T>>) -> F + 'static + Copy,
    ) where
        T: 'static,
        F: std::future::Future<Output = ()> + 'static,
    {
        let render_loop = fluffr_window.get_render_loop().unwrap();

        let window_ptr = FlufflWindowPtr {
            ptr: Arc::new(RefCell::new(fluffr_window)),
        };

        let aux_ptr = Rc::new(RefCell::new(aux));

        render_loop.run(move |running| {
            if window_ptr.window_mut_cb(|win| transfer_events(win)) {
                let win_ptr_clone = window_ptr.clone();
                let local_running = Rc::new(Cell::new(*running));
                let local_running_clone = local_running.clone();
                let aux_ptr_clone = aux_ptr.clone();
                spawn_local(core_loop(win_ptr_clone, local_running_clone, aux_ptr_clone));
                *running = local_running.get();
            }
        });
    }
}

impl WindowManager for FlufflWindow {
    #[allow(unused_variables)]
    /// spins up a window
    /// `config_xml` - an xml text containing window config data
    fn init(config_xml: &str) -> Result<Self, FlufflError> {
        determine_desktop_or_mobile();

        console_log!("is_mobile = {}\n", is_mobile());

        //get config settings from javascript (if possible)

        #[allow(unused_unsafe)]
        // let config_js = unsafe { get_xml_config() };

        // let xml_text = if config_js.is_empty() == false {
        //     config_js.as_str()
        // } else {
        //     config_xml
        // };
        let xml_text = config_xml;

        let settings = FlufflWindowConfigs::new().parser_config_file(config_xml);

        let web_window = web_sys::window().unwrap();
        let canvas = web_window
            .document()
            .unwrap()
            .get_element_by_id(settings.canvas_id.as_str())
            .unwrap()
            .dyn_into::<HtmlCanvasElement>()
            .unwrap();

        let webgl2_context = canvas
            .get_context(settings.webgl_version.as_str())
            .unwrap()
            .unwrap()
            .dyn_into::<WebGl2RenderingContext>()
            .unwrap();

        let gl = glow::Context::from_webgl2_context(webgl2_context);
        let render_loop = glow::RenderLoop::from_request_animation_frame();

        match attach_event_handlers(&web_window, &canvas) {
            Err(_) => {
                // console_write("Event handler instantiation failed!");
                return Err(FlufflError::WindowInitError(String::from(
                    "javascript event listeners failed",
                )));
            }
            _ => (),
        }
        let canvas = Rc::new(canvas);

        //I need a global reference to the canvas in this module
        init_global_canvas(canvas.clone());

        let window = Self {
            window_width: settings.width,
            window_height: settings.height,
            glue_event: Some(FlufflEvent::new()),
            render_loop: Some(render_loop),
            gl: Arc::new(Box::new(gl)),
            audio_ctx: Arc::new(RefCell::new(FlufflAudioContext::new())),
            canvas,
        };

        //I use this table to track touch displacements
        TouchTracker::init();

        //web implementation keeps a list of 'audio threads' in a static list
        init_audio_threads();

        //web implementation uses a temporary event queue from which the Fluffl window will read from
        init_global_event_queue();

        Ok(window)
    }

    fn get_events(&mut self) -> &mut FlufflEvent {
        self.glue_event.as_mut().unwrap()
    }
    fn width(&self) -> u32 {
        self.canvas.width()
    }
    fn height(&self) -> u32 {
        self.canvas.height()
    }

    fn gl(&self) -> Arc<Box<Context>> {
        self.gl.clone()
    }

    fn audio_context(&self) -> Arc<RefCell<FlufflAudioContext>> {
        self.audio_ctx.clone()
    }

    fn set_fullscreen(&mut self, go_fullscren: bool) {
        let document: Document = web_sys::window().unwrap().document().unwrap();

        if go_fullscren == true && document.fullscreen() == false {
            let canvas_ref = self.canvas.as_ref();
            let canvas_element: &HtmlElement = canvas_ref.dyn_ref::<HtmlElement>().unwrap();
            canvas_element
                .request_fullscreen()
                .expect("Fullscreen Failed");
        } else if go_fullscren == false && document.fullscreen() == true {
            document.exit_fullscreen();
        }
    }
}

fn attach_event_handlers(window: &Window, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    canvas.style().set_property("border", "solid")?;
    /*canvas resize handler*/
    {
        let closure = Closure::wrap(Box::new(move |_event: web_sys::UiEvent| {
            let event_queue = get_global_event_queue_mut();

            let (new_width, new_height) = {
                let canvas = get_canvas();
                (canvas.width() as i32, canvas.height() as i32)
            };

            event_queue.push_event(EventKind::Resize {
                width: new_width,
                height: new_height,
            });
        }) as Box<dyn FnMut(_)>);

        canvas.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
        window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    /*touch move handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::TouchEvent| {
            let touch_list: web_sys::TouchList = event.changed_touches();

            let event_queue = get_global_event_queue_mut();

            for k in 0..touch_list.length() {
                touch_list.item(k).map(|touch: Touch| {
                    let id = touch.identifier();

                    let x = touch.client_x() as f64;
                    let y = touch.client_y() as f64;
                    let (x, y, _, _) = convert_from_viewport_to_window(x, y, 0., 0.);
                    let (x, y) = (x as f32, y as f32);

                    let [dx, dy] = TouchTracker::get_touch_displacement(id, [x, y]);
                    event_queue.push_event(EventKind::TouchDown {
                        finger_id: id,
                        x,
                        y,
                        dx,
                        dy,
                    });
                });
            }
        }) as Box<dyn FnMut(_)>);

        canvas.add_event_listener_with_callback("touchmove", closure.as_ref().unchecked_ref())?;

        closure.forget();
    }

    /*touch end handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::TouchEvent| {
            let touch_list: web_sys::TouchList = event.touches();

            let event_queue = get_global_event_queue_mut();

            for k in 0..touch_list.length() {
                touch_list.item(k).map(|touch: Touch| {
                    let id = touch.identifier();
                    let x = touch.client_x() as f64;
                    let y = touch.client_y() as f64;

                    let (x, y, _, _) = convert_from_viewport_to_window(x, y, 0., 0.);
                    let (x, y) = (x as f32, y as f32);

                    TouchTracker::get_touch_displacement(id, [x, y]);

                    event_queue.push_event(EventKind::TouchUp {
                        finger_id: id,
                        x,
                        y,
                        dx: 0.,
                        dy: 0.,
                    });

                    // Its important to remove info associated with id when finger is released.
                    // We only want to track unique fingers detected by the touchscreen
                    TouchTracker::get_tracker_mut().table.remove(&id);
                });
            }
        }) as Box<dyn FnMut(_)>);

        canvas.add_event_listener_with_callback("touchend", closure.as_ref().unchecked_ref())?;

        closure.forget();
    }

    /*touch start handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::TouchEvent| {
            let touch_list: web_sys::TouchList = event.touches();

            let event_queue = get_global_event_queue_mut();

            for k in 0..touch_list.length() {
                touch_list.item(k).map(|touch: Touch| {
                    let id = touch.identifier();
                    let x = touch.client_x() as f64;
                    let y = touch.client_y() as f64;

                    let (x, y, _, _) = convert_from_viewport_to_window(x, y, 0., 0.);
                    let (x, y) = (x as f32, y as f32);

                    TouchTracker::get_touch_displacement(id, [x, y]);

                    event_queue.push_event(EventKind::TouchDown {
                        finger_id: id,
                        x,
                        y,
                        dx: 0.0,
                        dy: 0.0,
                    });
                });
            }
        }) as Box<dyn FnMut(_)>);

        canvas.add_event_listener_with_callback("touchstart", closure.as_ref().unchecked_ref())?;

        closure.forget();
    }

    /*mouse move handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let (x, y, dx, dy) = (
                event.client_x() as f64,
                event.client_y() as f64,
                event.movement_x() as f64,
                event.movement_y() as f64,
            );
            let (x, y, dx, dy) = convert_from_viewport_to_window(x, y, dx, dy);

            let event_queue = unsafe { GLOBAL_EVENT_QUEUE.as_mut().unwrap() };
            event_queue.push_event(EventKind::MouseMove { x, y, dx, dy });
        }) as Box<dyn FnMut(_)>);

        canvas.add_event_listener_with_callback("mousemove", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    /*mouse down handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let (x, y) = (event.client_x() as f64, event.client_y() as f64);

            let (x, y, _, _) = convert_from_viewport_to_window(x, y, 0., 0.);

            let button = event.button();

            let event_queue = unsafe { GLOBAL_EVENT_QUEUE.as_mut().unwrap() };
            event_queue.push_event(EventKind::MouseDown {
                button_code: get_button_code(button),
                x,
                y,
            });
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    /*mouse up handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let (x, y) = (event.client_x() as f64, event.offset_y() as f64);
            let button = event.button();

            let (x, y, _, _) = convert_from_viewport_to_window(x, y, 0., 0.);

            get_global_event_queue_mut().push_event(EventKind::MouseUp {
                button_code: get_button_code(button),
                x,
                y,
            });
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    /*context menu handler*/
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            //this is dont to prevent context menu from showing up
            event.prevent_default();
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("contextmenu", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    //wheel event listener
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::WheelEvent| {
            let delta_y = (-event.delta_y().signum()) as i32;
            get_global_event_queue_mut().push_event(EventKind::MouseWheel {
                button_code: MouseCode::WHEEL {
                    direction: delta_y as i32,
                },
            });
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("wheel", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    //add keydown to window
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            // console_log!("[down] key() = {} code = {}", event.key() , event.code() );
            let eq = get_global_event_queue_mut();
            let code = map_keycode(event.code().as_str());
            eq.push_event(EventKind::KeyDown { code })
        }) as Box<dyn FnMut(_)>);
        window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    //add keyup to window
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            // console_log!("[up] key() = {} code = {}", event.key() , event.code() );
            let eq = get_global_event_queue_mut();
            let code = map_keycode(event.code().as_str());
            eq.push_event(EventKind::KeyUp { code });
        }) as Box<dyn FnMut(_)>);
        window.add_event_listener_with_callback("keyup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    Ok(())
}

//event listeners fill GLOBAL and this function just transfers events
//from global to window
fn transfer_events(window: &mut FlufflWindow) {
    unsafe { GLOBAL_EVENT_QUEUE.as_mut() }
        .unwrap()
        .iter_mut()
        .for_each(|e| {
            window.get_events().push_event(e);
        });
}

fn get_button_code(button: i16) -> MouseCode {
    //maps javascript button codes to 'Glue' Codes
    match button {
        0 => MouseCode::LEFT_BUTTON,
        1 => MouseCode::WHEEL { direction: 0 },
        2 => MouseCode::RIGHT_BUTTON,
        _ => MouseCode::WHEEL { direction: 0 },
    }
}

fn get_global_event_queue_mut<'a>() -> &'a mut FlufflEvent {
    unsafe { GLOBAL_EVENT_QUEUE.as_mut().unwrap() }
}

// #[wasm_bindgen(module = "/js/aux_util.js")]
// extern "C" {
//     fn get_xml_config() -> String;
//     /// This kind of acts as "command line arguments" for the wasm module.
//     /// The idea is that you call `set_xml_config` right before you *init* the wasm module and
//     /// whatever config options get passed through this function will take absolute precedence.
//     pub fn set_xml_config(xml_text: String);
// }

/// # Description
/// converts javascript viewport coordinates to expected window coordinates
/// # Retuns
/// A 4-tuple of the format: `( x,y,dx,dy )`
/// # Comments
/// I hate that I had to write this routine. Either the Javascript coordinate system sucks
/// Or the documentation is lacking something.
fn convert_from_viewport_to_window(x: f64, y: f64, dx: f64, dy: f64) -> (i32, i32, i32, i32) {
    if is_mobile() {
        viewport_to_window_mobile_browser(x, y, dx, dy)
    } else {
        viewport_to_window_desktop_browser(x, y, dx, dy)
    }
}

fn viewport_to_window_mobile_browser(x: f64, y: f64, dx: f64, dy: f64) -> (i32, i32, i32, i32) {
    if is_in_portrait_mode() {
        //desktop way works in portrait mode both fullscreen and
        viewport_to_window_desktop_browser(x, y, dx, dy)
    } else {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let screen: Screen = window.screen().ok().unwrap();
        let canvas = get_canvas();
        let rect = canvas.get_bounding_client_rect();

        let canvas_width = canvas.width() as f64;
        let canvas_height = canvas.height() as f64;

        if document.fullscreen() {
            let aspect_ratio = canvas_width / canvas_height;
            let vp_height = rect.height();
            let vp_width = aspect_ratio * vp_height;
            let vp_x = rect.width() / 2.0 - vp_width / 2.0;
            let vp_y = 0.0;

            let x = (x - vp_x) * canvas_width / vp_width;
            let y = (y - vp_y) * (canvas_height) / vp_height;
            let dx = (dx - vp_x) * canvas_width / vp_width;
            let dy = (dy - vp_y) * (canvas_height) / vp_height;

            (x as i32, y as i32, dx as i32, dy as i32)
        } else {
            let sx = canvas_width / rect.width() as f64;
            let sy = canvas_height / rect.height() as f64;
            let x = (x * sx - rect.x() * sx) as i32;
            let y = (y * sy - rect.y() * sy) as i32;
            let dx = (dx * sx - rect.x() * sx) as i32;
            let dy = (dy * sx - rect.y() * sx) as i32;

            // console_log!(
            //     "pos:[{},{}],vp_dims: [{},{}], rect dims: [x:{},y:{},w:{},h:{}], screen:[aw:{},ah:{},w:{},h:{}]\n",
            //     x,
            //     y,
            //     -1,
            //     -1,
            //     rect.x(),
            //     rect.y(),
            //     rect.width(),
            //     rect.height(),
            //     screen.avail_width().ok().unwrap(),
            //     screen.avail_height().ok().unwrap(),
            //     screen.width().ok().unwrap(),
            //     screen.height().ok().unwrap(),
            //     );

            (x, y, dx, dy)
        }
    }
}

fn viewport_to_window_desktop_browser(x: f64, y: f64, dx: f64, dy: f64) -> (i32, i32, i32, i32) {
    let canvas = get_canvas();
    let rect = canvas.get_bounding_client_rect();
    let document: Document = web_sys::window().unwrap().document().unwrap();

    let canvas_width = canvas.width() as f64;
    let canvas_height = canvas.height() as f64;

    if document.fullscreen() == false {
        let sx = canvas_width / rect.width() as f64;
        let sy = canvas_height / rect.height() as f64;
        let x = (x * sx - rect.x() * sx) as i32;
        let y = (y * sy - rect.y() * sy) as i32;
        let dx = (dx * sx - rect.x() * sx) as i32;
        let dy = (dy * sx - rect.y() * sx) as i32;
        (x, y, dx, dy)
    } else {
        // Explanation for the Else check:
        // The else case has to be checked because I noticed that,when in fullscreen,
        // the get_bounding_client_rect() AABB doesn't actually cover the desired region in fullscreen.
        // So I had to compute the AABB manually knowing that firefox preserves aspect ratio and that AABB in fullscreen is just the
        // clients screen dimensions
        // console_log!("bounding_client:[{},{}]\n", rect.width(), rect.height());

        let aspect_ratio = canvas_width / canvas_height;

        let (vp_width, vp_height, vp_x, vp_y) = if rect.width() >= canvas_width {
            let vp_width = rect.height() * aspect_ratio;
            let vp_height = rect.height();
            let vp_x = rect.width() / 2. - vp_width / 2.0;
            let vp_y = 0.0;
            (vp_width, vp_height, vp_x, vp_y)
        } else {
            //this mapping is broken for modile (fullscreen+landscape)
            let vp_height = rect.width() / aspect_ratio;
            let vp_width = rect.width();
            let vp_x = 0.0;
            let vp_y = rect.height() / 2. - vp_height / 2.0;
            (vp_width, vp_height, vp_x, vp_y)
        };

        // compute transformation matrix
        // [a,b]
        // [c,d]
        // let sx = canvas_width/vp_width
        // let sy = canvas_height/vp_height
        // then this:
        // let x = ((x - vp_x) * (canvas.width() as f64 /vp_width) )  as i32;
        // let y = ((y - vp_y) *(canvas.height() as f64/ vp_height))  as i32;
        // becomes this:
        // let x = x*sx -vp_x*sx
        // let y = y*sy -vp_y*sy
        // becomes(in matrix form):
        // [x'] = [sx][-vp_x*sx]  [x]
        // [y'] = [sy][-yp_y*sy]  [y]

        let sx = canvas.width() as f64 / vp_width;
        let sy = canvas.height() as f64 / vp_height;

        let x = (sx * x - vp_x * sx) as i32;
        let y = (sy * y - vp_y * sy) as i32;
        let dx = (dx * sx - vp_x * sx) as i32;
        let dy = (dy * sy - vp_y * sy) as i32;

        (x, y, dx, dy)
    }
}

/// # Description
/// checks if the browser is in portrait mode
fn is_in_portrait_mode() -> bool {
    let window = web_sys::window().unwrap();
    if let Ok(Some(query_list)) = window.match_media("(orientation: portrait)") {
        query_list.matches()
    } else {
        false
    }
}

fn map_keycode(js_keycode: &str) -> KeyCode {
    match js_keycode {
        "KeyA" => KeyCode::KEY_A,
        "KeyB" => KeyCode::KEY_B,
        "KeyC" => KeyCode::KEY_C,
        "KeyD" => KeyCode::KEY_D,
        "KeyE" => KeyCode::KEY_E,
        "KeyF" => KeyCode::KEY_F,
        "KeyG" => KeyCode::KEY_G,
        "KeyH" => KeyCode::KEY_H,
        "KeyI" => KeyCode::KEY_I,
        "KeyJ" => KeyCode::KEY_J,
        "KeyK" => KeyCode::KEY_K,
        "KeyL" => KeyCode::KEY_L,
        "KeyM" => KeyCode::KEY_M,
        "KeyN" => KeyCode::KEY_N,
        "KeyO" => KeyCode::KEY_O,
        "KeyP" => KeyCode::KEY_P,
        "KeyQ" => KeyCode::KEY_Q,
        "KeyR" => KeyCode::KEY_R,
        "KeyS" => KeyCode::KEY_S,
        "KeyT" => KeyCode::KEY_T,
        "KeyU" => KeyCode::KEY_U,
        "KeyV" => KeyCode::KEY_V,
        "KeyW" => KeyCode::KEY_W,
        "KeyX" => KeyCode::KEY_X,
        "KeyY" => KeyCode::KEY_Y,
        "KeyZ" => KeyCode::KEY_Z,
        "Backquote" => KeyCode::BACK_QUOTE,
        "Digit0" => KeyCode::NUM_0,
        "Digit1" => KeyCode::NUM_1,
        "Digit2" => KeyCode::NUM_2,
        "Digit3" => KeyCode::NUM_3,
        "Digit4" => KeyCode::NUM_4,
        "Digit5" => KeyCode::NUM_5,
        "Digit6" => KeyCode::NUM_6,
        "Digit7" => KeyCode::NUM_7,
        "Digit8" => KeyCode::NUM_8,
        "Digit9" => KeyCode::NUM_9,
        "Minus" => KeyCode::MINUS,
        "Equal" => KeyCode::EQUALS,
        "Comma" => KeyCode::COMMA,
        "Semicolon" => KeyCode::COLON,
        "Quote" => KeyCode::QUOTE,
        "Slash" => KeyCode::FORWARD_SLASH,
        "Backslash" => KeyCode::BACKSLASH,
        "Insert" => KeyCode::INSERT,
        "Home" => KeyCode::HOME,
        "PageUp" => KeyCode::PAGE_UP,
        "PageDown" => KeyCode::PAGE_DOWN,
        "End" => KeyCode::END,
        "Delete" => KeyCode::DELETE,
        "ShiftLeft" => KeyCode::SHIFT_L,
        "ShiftRight" => KeyCode::SHIFT_R,
        "ArrowUp" => KeyCode::ARROW_UP,
        "ArrowLeft" => KeyCode::ARROW_LEFT,
        "ArrowRight" => KeyCode::ARROW_RIGHT,
        "ArrowDown" => KeyCode::ARROW_DOWN,
        "Space" => KeyCode::SPACE,
        "Period" => KeyCode::PERIOD,
        _ => KeyCode::UNKNOWN,
    }
}
