use super::{GlueAudioDeviceCore, IntoWithArg};

use wasm_bindgen::prelude::*;
use wasm_bindgen::*;
use wasm_bindgen_futures::*;
use web_sys::*;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use crate::collections::linked_list::*;
use crate::console::*;
use crate::*;

static mut AUDIO_THREADS: Option<LinkedList<Option<js_sys::Function>>> = None;

/// Inits a gobally managed pool of 'audio threads'(Theese threads are NOT executed concurrently).\
/// They are executed asynconously. These javascript functions are put on either the microtask or task queue of the hosts javascript engine\
/// When buffering or sound processing is needed the browser will allocated  allocated a time-slice for these functions to be executed asyncronously.
pub fn init_audio_threads() {
    unsafe {
        AUDIO_THREADS = Some(LinkedList::new());
    }
}

fn get_audio_thread_list() -> &'static LinkedList<Option<js_sys::Function>> {
    unsafe { AUDIO_THREADS.as_ref().unwrap() }
}

fn get_audio_thread_list_mut() -> &'static mut LinkedList<Option<js_sys::Function>> {
    unsafe { AUDIO_THREADS.as_mut().unwrap() }
}

pub struct GlueAudioContext {
    pub ctx: AudioContext,
}

impl GlueAudioContext {
    pub fn new() -> Self {
        let ctx = AudioContext::new().unwrap();
        Self { ctx }
    }
}

impl Drop for GlueAudioContext {
    fn drop(&mut self) {
        let _ = self.ctx.close();
    }
}

pub struct GlueAudioDeviceContext<F, S> {
    glue_callback: F,
    state: Rc<RefCell<S>>,
    thread_id: u32,
}

impl<F, S> Clone for GlueAudioDeviceContext<F, S>
where
    F: Copy,
{
    fn clone(&self) -> Self {
        Self {
            glue_callback: self.glue_callback,
            state: self.state.clone(),
            thread_id: self.thread_id,
        }
    }
}

impl<F, S> GlueAudioDeviceContext<F, S>
where
    F: FnMut(&mut S, &mut [f32]) + Copy  + Send +'static,
    S:  'static,
{
    pub fn new(
        mut core: GlueAudioDeviceCore<F, S>,
        audio_context: Arc<RefCell<GlueAudioContext>>,
    ) -> Self {
        // the time (in seconds) to buffer ahead to avoid choppy playback
        const BUFFER_TIME: f64 = 1.0;

        let pump_list: Rc<RefCell<VecDeque<js_sys::Function>>> =
            Rc::new(RefCell::new(VecDeque::new()));

        let state = core.state.take().unwrap_or_else(|| {
            panic!("Error: Failed to create GlueAudioDevice!\n .with_state(..) not initalized!\n")
        });

        let mut glue_callback = core.callback();
        let state = Rc::new(RefCell::new(state));
        let context_state = state.clone();
        let (sample_rate, channels, buffer_size) = core.desired_specs.get_specs();
        let mut play_time = audio_context.borrow().ctx.current_time();

        console_log!("[{},{},{}]", sample_rate, channels, buffer_size);

        //this buffer contains INTERLEAVED pcm in 32-bit IEEE-754 floating point precision
        //however the webaudio api demanands each channel be submitted seperately,so this routine will have
        //to manually split the PCM codes after glue_callback(...) is called.
        let mut sample_callback_buffer = Vec::new();
        let mut sample_buffer_for_channel = Vec::new();

        //make sure that allocation happens up front
        sample_buffer_for_channel.resize(buffer_size, 0f32);

        //allocate uninitalized audio 'thread' to front of linked list
        get_audio_thread_list_mut().push_front(None);
        // get pointer to the front
        let thread_id = get_audio_thread_list().get_front();

        // let mut t = play_time;
        // let delta = 1.0 / sample_rate as f64;

        let process_raw_pcm = move || {
            // console_log!("play time= {}",play_time);

            //buffer for one second into the future
            while play_time - audio_context.borrow().ctx.current_time() < BUFFER_TIME {
                let web_audio_buffer = audio_context
                    .borrow()
                    .ctx
                    .create_buffer(channels as u32, buffer_size as u32, sample_rate as f32)
                    .unwrap();

                let mut web_audio_samples = web_audio_buffer.get_channel_data(0).unwrap();

                //clear buffers before calling the callback
                sample_callback_buffer.resize(buffer_size * channels, 0f32);

                //call the callback provided by the user
                glue_callback(&mut *state.borrow_mut(), &mut sample_callback_buffer[..]);

                //de-interleave samples into a buffer with samples associated with just a single channel
                for channel_index in 0..channels {
                    //clear the buffer holding PCM for a specific channel
                    sample_buffer_for_channel.clear();

                    //collect samples for channel 'channel_index'
                    for k in (0..sample_callback_buffer.len() / channels) {
                        let sample_index = k * channels + channel_index;
                        sample_buffer_for_channel.push(sample_callback_buffer[sample_index]);
                    }

                    //copy the callback buffer to the web_audio_buffer
                    web_audio_buffer
                        .copy_to_channel(&mut sample_buffer_for_channel[..], channel_index as i32)
                        .unwrap();
                }

                let web_audio_buffer_source_node =
                    audio_context.borrow().ctx.create_buffer_source().unwrap();

                web_audio_buffer_source_node.set_buffer(Some(&web_audio_buffer));

                let pump_list_ptr = pump_list.clone();
                // This function is fired whenever 'onended' event is fired
                // continue_buffering() literally just resumes the 'thread'
                let continue_buffering = move || {
                    let pump_list = pump_list_ptr;
                    let thread: &js_sys::Function = get_audio_thread_list()[thread_id]
                        .get_data()
                        .as_ref()
                        .unwrap();

                    thread.call0(&JsValue::null());

                    // because we know 'onended' had to have fired  we can remove the oldest buffer
                    // this buffer is likely to be garbage collected by the JS interpreter
                    pump_list.borrow_mut().pop_front();
                    // console_log!(
                    //     "callback triggered, pl size = {}",
                    //     pump_list.borrow().len()
                    // );
                };

                //wrap continue_buffering in boxed closure and convert to a Js Function
                let cb = Closure::once_into_js(continue_buffering)
                    .dyn_into::<js_sys::Function>()
                    .unwrap();

                //push the continue_buffering callback into a queue so JS doesn't collect it as garbage
                pump_list.borrow_mut().push_back(cb);

                // when this buffer finished playing resume buffering
                web_audio_buffer_source_node.set_onended(pump_list.borrow().back());

                // play the buffer at time t=play_time
                web_audio_buffer_source_node
                    .start_with_when(play_time)
                    .unwrap();

                play_time += buffer_size as f64 / sample_rate as f64;

                let node: AudioNode = web_audio_buffer_source_node
                    .dyn_into::<AudioNode>()
                    .unwrap();

                node.connect_with_audio_node(
                    &audio_context.borrow().ctx.destination().dyn_into().unwrap(),
                );
            }
        };

        // The audio 'thread' get initalized here
        let process_raw_pcm_closure = Closure::wrap(Box::new(process_raw_pcm) as Box<dyn FnMut()>)
            .into_js_value()
            .dyn_into::<js_sys::Function>()
            .unwrap();

        //initalize 'audio thread'
        *get_audio_thread_list_mut()[thread_id].get_data_mut() = Some(process_raw_pcm_closure);

        Self {
            state: context_state,
            glue_callback,
            thread_id,
        }
    }

    pub fn modify_state<CBF>(&self, mut cb: CBF)
    where
        CBF: FnMut(Option<&mut S>),
    {
        if let Ok(mut state_ptr)  =self.state.try_borrow_mut(){
            let state_ref = &mut *state_ptr;
            cb(Some(state_ref));
        }
    }

    pub fn resume(&self) {
        let thread_id = self.thread_id;
        //begin 'audio thread'
        get_audio_thread_list_mut()[thread_id]
            .get_data()
            .as_ref()
            .unwrap()
            .call0(&JsValue::null());
    }

    pub fn pause(&self) {
        panic!("not implemented");
    }
}

pub struct GlueAudioDevice<F, S> {
    core: GlueAudioDeviceCore<F, S>,
}

impl<F, S> GlueAudioDevice<F, S>
where
    F: FnMut(&mut S, &mut [f32]) + std::marker::Copy + Send,
    S: Send,
{
    pub fn callback(&self) -> F {
        panic!("not implemented");
    }

    pub fn state(&mut self) -> Option<&mut S> {
        panic!("not implemented");
    }
}

impl<F, S> IntoWithArg<GlueAudioDeviceContext<F, S>, Arc<RefCell<GlueAudioContext>>>
    for GlueAudioDeviceCore<F, S>
where
    F: FnMut(&mut S, &mut [f32]) + Send + std::marker::Copy + 'static,
    S: Send + 'static,
{
    fn into_with(self, arg: Arc<RefCell<GlueAudioContext>>) -> GlueAudioDeviceContext<F, S> {
        GlueAudioDeviceContext::new(self, arg)
    }
}

// playing around  with the api
// sample_buffer_for_channel.iter_mut().enumerate().for_each(|(i,e)|{
//     let dt =  i as f64 /sample_rate as f64 ;
//     let t = play_time +dt ;
//     *e = (t * 1440.0 ).sin() as f32;
// });

// web_audio_buffer
//     .copy_to_channel(&mut sample_buffer_for_channel[..], 0 as i32)
//     .unwrap();

// sample_buffer_for_channel.iter_mut().enumerate().for_each(|(i,e)|{
//     let dt =  i as f64 /sample_rate as f64 ;
//     let t = play_time +dt ;
//     *e = (t * 440.0 ).sin() as f32;
// });

// web_audio_buffer
//     .copy_to_channel(&mut sample_buffer_for_channel[..], 1 as i32)
//     .unwrap();

// console_log!("demux skipped");
