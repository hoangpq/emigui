use {
    js_sys::WebAssembly,
    wasm_bindgen::{prelude::*, JsCast},
    web_sys::{WebGlBuffer, WebGlProgram, WebGlRenderingContext, WebGlShader, WebGlTexture},
};

use egui::{
    paint::{Color, PaintBatches, Texture, Triangles},
    vec2, Pos2,
};

type Gl = WebGlRenderingContext;

pub struct Painter {
    canvas: web_sys::HtmlCanvasElement,
    gl: WebGlRenderingContext,
    texture: WebGlTexture,
    program: WebGlProgram,
    index_buffer: WebGlBuffer,
    pos_buffer: WebGlBuffer,
    tc_buffer: WebGlBuffer,
    color_buffer: WebGlBuffer,
    tex_size: (u16, u16),
    current_texture_id: Option<u64>,
}

impl Painter {
    pub fn debug_info(&self) -> String {
        format!(
            "Stored canvas size: {} x {}\n\
             gl context size: {} x {}",
            self.canvas.width(),
            self.canvas.height(),
            self.gl.drawing_buffer_width(),
            self.gl.drawing_buffer_height(),
        )
    }

    pub fn new(canvas_id: &str) -> Result<Painter, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document.get_element_by_id(canvas_id).unwrap();
        let canvas: web_sys::HtmlCanvasElement = canvas.dyn_into::<web_sys::HtmlCanvasElement>()?;

        let gl = canvas
            .get_context("webgl")?
            .unwrap()
            .dyn_into::<WebGlRenderingContext>()?;

        // --------------------------------------------------------------------

        let gl_texture = gl.create_texture().unwrap();
        gl.bind_texture(Gl::TEXTURE_2D, Some(&gl_texture));
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::LINEAR as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::LINEAR as i32);

        // --------------------------------------------------------------------

        let vert_shader = compile_shader(
            &gl,
            Gl::VERTEX_SHADER,
            r#"
            uniform vec4 u_clip_rect; // min_x, min_y, max_x, max_y
            uniform vec2 u_screen_size;
            uniform vec2 u_tex_size;
            attribute vec2 a_pos;
            attribute vec2 a_tc;
            attribute vec4 a_color;
            varying vec2 v_pos;
            varying vec4 v_color;
            varying vec2 v_tc;
            varying vec4 v_clip_rect;
            void main() {
                gl_Position = vec4(
                    2.0 * a_pos.x / u_screen_size.x - 1.0,
                    1.0 - 2.0 * a_pos.y / u_screen_size.y,
                    0.0,
                    1.0);
                v_pos = a_pos;
                v_color = a_color;
                v_tc = a_tc / u_tex_size;
                v_clip_rect = u_clip_rect;
            }
        "#,
        )?;
        let frag_shader = compile_shader(
            &gl,
            Gl::FRAGMENT_SHADER,
            r#"
            uniform sampler2D u_sampler;
            precision highp float;
            varying vec2 v_pos;
            varying vec4 v_color;
            varying vec2 v_tc;
            varying vec4 v_clip_rect;
            void main() {
                if (v_pos.x < v_clip_rect.x) { discard; }
                if (v_pos.y < v_clip_rect.y) { discard; }
                if (v_pos.x > v_clip_rect.z) { discard; }
                if (v_pos.y > v_clip_rect.w) { discard; }
                gl_FragColor = v_color;
                gl_FragColor *= texture2D(u_sampler, v_tc).a;
            }
        "#,
        )?;
        let program = link_program(&gl, [vert_shader, frag_shader].iter())?;
        let index_buffer = gl.create_buffer().ok_or("failed to create index_buffer")?;
        let pos_buffer = gl.create_buffer().ok_or("failed to create pos_buffer")?;
        let tc_buffer = gl.create_buffer().ok_or("failed to create tc_buffer")?;
        let color_buffer = gl.create_buffer().ok_or("failed to create color_buffer")?;

        Ok(Painter {
            canvas,
            gl,
            texture: gl_texture,
            program,
            index_buffer,
            pos_buffer,
            tc_buffer,
            color_buffer,
            tex_size: (0, 0),
            current_texture_id: None,
        })
    }

    fn upload_texture(&mut self, texture: &Texture) {
        if self.current_texture_id == Some(texture.id) {
            return; // No change
        }

        let gl = &self.gl;
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.texture));

        let level = 0;
        let internal_format = Gl::ALPHA;
        let border = 0;
        let src_format = Gl::ALPHA;
        let src_type = Gl::UNSIGNED_BYTE;
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            Gl::TEXTURE_2D,
            level,
            internal_format as i32,
            texture.width as i32,
            texture.height as i32,
            border,
            src_format,
            src_type,
            Some(&texture.pixels),
        )
        .unwrap();

        self.tex_size = (texture.width as u16, texture.height as u16);
        self.current_texture_id = Some(texture.id);
    }

    pub fn paint_batches(
        &mut self,
        bg_color: Color,
        batches: PaintBatches,
        texture: &Texture,
        pixels_per_point: f32,
    ) -> Result<(), JsValue> {
        self.upload_texture(texture);

        let gl = &self.gl;

        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::ONE, Gl::ONE_MINUS_SRC_ALPHA); // premultiplied alpha
        gl.use_program(Some(&self.program));
        gl.active_texture(Gl::TEXTURE0);
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.texture));

        let u_clip_rect_loc = gl
            .get_uniform_location(&self.program, "u_clip_rect")
            .unwrap();

        let u_screen_size_loc = gl
            .get_uniform_location(&self.program, "u_screen_size")
            .unwrap();
        let screen_size_pixels = vec2(self.canvas.width() as f32, self.canvas.height() as f32);
        let screen_size_points = screen_size_pixels / pixels_per_point;
        gl.uniform2f(
            Some(&u_screen_size_loc),
            screen_size_points.x,
            screen_size_points.y,
        );

        let u_tex_size_loc = gl
            .get_uniform_location(&self.program, "u_tex_size")
            .unwrap();
        gl.uniform2f(
            Some(&u_tex_size_loc),
            f32::from(self.tex_size.0),
            f32::from(self.tex_size.1),
        );

        let u_sampler_loc = gl.get_uniform_location(&self.program, "u_sampler").unwrap();
        gl.uniform1i(Some(&u_sampler_loc), 0);

        gl.viewport(
            0,
            0,
            self.canvas.width() as i32,
            self.canvas.height() as i32,
        );
        gl.clear_color(
            bg_color.r as f32 / 255.0,
            bg_color.g as f32 / 255.0,
            bg_color.b as f32 / 255.0,
            bg_color.a as f32 / 255.0,
        );
        gl.clear(Gl::COLOR_BUFFER_BIT);

        for (clip_rect, triangles) in batches {
            // Avoid infinities in shader:
            let clip_min = clip_rect.min.max(Pos2::default());
            let clip_max = clip_rect.max.min(Pos2::default() + screen_size_points);

            gl.uniform4f(
                Some(&u_clip_rect_loc),
                clip_min.x,
                clip_min.y,
                clip_max.x,
                clip_max.y,
            );

            for triangles in triangles.split_to_u16() {
                self.paint_triangles(&triangles)?;
            }
        }
        Ok(())
    }

    fn paint_triangles(&self, triangles: &Triangles) -> Result<(), JsValue> {
        let indices: Vec<u16> = triangles.indices.iter().map(|idx| *idx as u16).collect();

        let mut positions: Vec<f32> = Vec::with_capacity(2 * triangles.vertices.len());
        let mut tex_coords: Vec<u16> = Vec::with_capacity(2 * triangles.vertices.len());
        for v in &triangles.vertices {
            positions.push(v.pos.x);
            positions.push(v.pos.y);
            tex_coords.push(v.uv.0);
            tex_coords.push(v.uv.1);
        }

        let mut colors: Vec<u8> = Vec::with_capacity(4 * triangles.vertices.len());
        for v in &triangles.vertices {
            colors.push(v.color.r);
            colors.push(v.color.g);
            colors.push(v.color.b);
            colors.push(v.color.a);
        }

        // --------------------------------------------------------------------

        let gl = &self.gl;

        let indices_memory_buffer = wasm_bindgen::memory()
            .dyn_into::<WebAssembly::Memory>()?
            .buffer();
        let indices_ptr = indices.as_ptr() as u32 / 2;
        let indices_array = js_sys::Int16Array::new(&indices_memory_buffer)
            .subarray(indices_ptr, indices_ptr + indices.len() as u32);

        gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&self.index_buffer));
        gl.buffer_data_with_array_buffer_view(
            Gl::ELEMENT_ARRAY_BUFFER,
            &indices_array,
            Gl::STREAM_DRAW,
        );

        // --------------------------------------------------------------------

        let pos_memory_buffer = wasm_bindgen::memory()
            .dyn_into::<WebAssembly::Memory>()?
            .buffer();
        let pos_ptr = positions.as_ptr() as u32 / 4;
        let pos_array = js_sys::Float32Array::new(&pos_memory_buffer)
            .subarray(pos_ptr, pos_ptr + positions.len() as u32);

        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.pos_buffer));
        gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &pos_array, Gl::STREAM_DRAW);

        let a_pos_loc = gl.get_attrib_location(&self.program, "a_pos");
        assert!(a_pos_loc >= 0);
        let a_pos_loc = a_pos_loc as u32;

        let normalize = false;
        let stride = 0;
        let offset = 0;
        gl.vertex_attrib_pointer_with_i32(a_pos_loc, 2, Gl::FLOAT, normalize, stride, offset);
        gl.enable_vertex_attrib_array(a_pos_loc);

        // --------------------------------------------------------------------

        let tc_memory_buffer = wasm_bindgen::memory()
            .dyn_into::<WebAssembly::Memory>()?
            .buffer();
        let tc_ptr = tex_coords.as_ptr() as u32 / 2;
        let tc_array = js_sys::Uint16Array::new(&tc_memory_buffer)
            .subarray(tc_ptr, tc_ptr + tex_coords.len() as u32);

        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.tc_buffer));
        gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &tc_array, Gl::STREAM_DRAW);

        let a_tc_loc = gl.get_attrib_location(&self.program, "a_tc");
        assert!(a_tc_loc >= 0);
        let a_tc_loc = a_tc_loc as u32;

        let normalize = false;
        let stride = 0;
        let offset = 0;
        gl.vertex_attrib_pointer_with_i32(
            a_tc_loc,
            2,
            Gl::UNSIGNED_SHORT,
            normalize,
            stride,
            offset,
        );
        gl.enable_vertex_attrib_array(a_tc_loc);

        // --------------------------------------------------------------------

        let colors_memory_buffer = wasm_bindgen::memory()
            .dyn_into::<WebAssembly::Memory>()?
            .buffer();
        let colors_ptr = colors.as_ptr() as u32;
        let colors_array = js_sys::Uint8Array::new(&colors_memory_buffer)
            .subarray(colors_ptr, colors_ptr + colors.len() as u32);

        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.color_buffer));
        gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &colors_array, Gl::STREAM_DRAW);

        let a_color_loc = gl.get_attrib_location(&self.program, "a_color");
        assert!(a_color_loc >= 0);
        let a_color_loc = a_color_loc as u32;

        let normalize = true;
        let stride = 0;
        let offset = 0;
        gl.vertex_attrib_pointer_with_i32(
            a_color_loc,
            4,
            Gl::UNSIGNED_BYTE,
            normalize,
            stride,
            offset,
        );
        gl.enable_vertex_attrib_array(a_color_loc);

        // --------------------------------------------------------------------

        gl.draw_elements_with_i32(Gl::TRIANGLES, indices.len() as i32, Gl::UNSIGNED_SHORT, 0);

        Ok(())
    }
}

fn compile_shader(
    gl: &WebGlRenderingContext,
    shader_type: u32,
    source: &str,
) -> Result<WebGlShader, String> {
    let shader = gl
        .create_shader(shader_type)
        .ok_or_else(|| String::from("Unable to create shader object"))?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);

    if gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(shader)
    } else {
        Err(gl
            .get_shader_info_log(&shader)
            .unwrap_or_else(|| "Unknown error creating shader".into()))
    }
}

fn link_program<'a, T: IntoIterator<Item = &'a WebGlShader>>(
    gl: &WebGlRenderingContext,
    shaders: T,
) -> Result<WebGlProgram, String> {
    let program = gl
        .create_program()
        .ok_or_else(|| String::from("Unable to create shader object"))?;
    for shader in shaders {
        gl.attach_shader(&program, shader)
    }
    gl.link_program(&program);

    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(program)
    } else {
        Err(gl
            .get_program_info_log(&program)
            .unwrap_or_else(|| "Unknown error creating program object".into()))
    }
}
