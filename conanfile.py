from conans import ConanFile, CMake

class EvoBenchConan(ConanFile):
    name = "evobench"
    version = "0.1"
    license = "MIT"
    author = "Christian Jaeger <christian.jaeger@bsse.ethz.ch>"
    url = "https://github.com/GenSpectrum/evobench/"
    description = "cEvo's performance benchmarking library currently for C++"
    topics = ("evolution", "benchmark", "genetics")
    settings = "os", "compiler", "build_type", "arch"
    generators = "cmake"
    
    # Exports all files in the cpp directory
    exports_sources = "cpp/*"
    
    def build(self):
        cmake = CMake(self)
        # Point to the cpp folder where your CMakeLists.txt is located.
        cmake.configure(source_folder="cpp")
        cmake.build()
    
    def package(self):
        # Copy header files.
        self.copy("*.hpp", dst="include", src="cpp/include")
        # Copy the built library files (adjust patterns for static/shared libraries).
        self.copy("*.a", dst="lib", keep_path=False)
        self.copy("*.so", dst="lib", keep_path=False)
        #self.copy("*.dll", dst="bin", keep_path=False)
        self.copy("*.lib", dst="lib", keep_path=False)
    
    def package_info(self):
        # Consumers will link against 'evobench'
        self.cpp_info.libs = ["evobench"]
